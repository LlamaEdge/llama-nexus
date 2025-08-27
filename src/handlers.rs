use std::{sync::Arc, time::SystemTime};

use axum::{
    Json,
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, Response, StatusCode},
};
use endpoints::{
    chat::{ChatCompletionRequest, ChatCompletionRequestMessage, Tool, ToolChoice, ToolFunction},
    embeddings::EmbeddingRequest,
    models::{ListModelsResponse, Model},
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::{
    AppState,
    chat::gen_chat_id,
    config::ChatMode,
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    info::ApiServer,
    server::{RoutingPolicy, Server, ServerIdToRemove, ServerKind},
};

pub(crate) async fn chat_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    // check if the user id is provided
    if request.user.is_none() {
        request.user = Some(gen_chat_id());
    };
    dual_info!(
        "Received a new chat request from user: {} - request_id: {}",
        request.user.as_ref().unwrap(),
        request_id
    );

    // update the request with MCP tools
    if let Some(mcp_config) = state.config.read().await.mcp.as_ref()
        && !mcp_config.server.tool_servers.is_empty()
    {
        dual_info!("Updating the request with MCP tools");

        let mut more_tools = Vec::new();
        for server_config in mcp_config.server.tool_servers.iter() {
            if server_config.enable {
                server_config
                    .tools
                    .as_ref()
                    .unwrap()
                    .iter()
                    .for_each(|mcp_tool| {
                        let tool = Tool::new(ToolFunction {
                            name: mcp_tool.name.to_string(),
                            description: mcp_tool.description.as_ref().map(|s| s.to_string()),
                            parameters: Some((*mcp_tool.input_schema).clone()),
                        });

                        more_tools.push(tool.clone());
                    });
            }
        }

        if !more_tools.is_empty() {
            if let Some(tools) = &mut request.tools {
                tools.extend(more_tools);
            } else {
                request.tools = Some(more_tools);
            }

            // set the tool choice to auto
            if let Some(ToolChoice::None) | None = request.tool_choice {
                request.tool_choice = Some(ToolChoice::Auto);
            }
        }
    }

    // Create or get conversation ID for memory
    let conv_id = if let Some(memory) = &state.memory {
        if let Some(user) = &request.user {
            // 使用全局持久化的对话管理：同一用户无论使用什么模型都复用同一个对话
            let model_name = request
                .model
                .clone()
                .unwrap_or_else(|| "default".to_string());
            match memory
                .get_or_create_user_conversation(user, &model_name)
                .await
            {
                Ok(id) => {
                    dual_debug!(
                        "Using conversation {} for user {} - request_id: {}",
                        id,
                        user,
                        request_id
                    );
                    Some(id)
                }
                Err(e) => {
                    dual_warn!(
                        "Failed to get or create conversation for user {}: {} - request_id: {}",
                        user,
                        e,
                        request_id
                    );
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Get chat mode from configuration
    let chat_mode = {
        let config = state.config.read().await;
        config.server.chat_mode
    };
    dual_debug!(
        "Using chat mode: {:?} - request_id: {}",
        chat_mode,
        request_id
    );

    // Route to appropriate chat handler based on configuration
    let res = match chat_mode {
        ChatMode::Normal => {
            crate::chat::normal::chat(
                State(state.clone()),
                Extension(cancel_token),
                headers,
                Json(request),
                conv_id.clone(),
                &request_id,
            )
            .await
        }
        ChatMode::React => {
            crate::chat::react::chat(
                State(state.clone()),
                Extension(cancel_token),
                headers,
                Json(request),
                conv_id.clone(),
                &request_id,
            )
            .await
        }
    };

    // Print chat history
    if let Some(memory) = &state.memory
        && let Some(conv_id) = &conv_id
    {
        let chat_history = memory.get_full_history(conv_id, true).await.map_err(|e| {
            let err_msg = format!("Failed to get chat history: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;
        dual_debug!(
            "🔍 Full history - request_id: {}\n{}",
            request_id,
            serde_json::to_string_pretty(&chat_history).unwrap()
        );

        let working_messages = memory.get_working_messages(conv_id).await.map_err(|e| {
            let err_msg = format!("Failed to get working messages: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;
        dual_debug!(
            "🔍 Working messages - request_id: {}\n{}",
            request_id,
            serde_json::to_string_pretty(&working_messages).unwrap()
        );

        let context = memory.get_model_context(conv_id).await.map_err(|e| {
            let err_msg = format!("Failed to get model context: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;
        let context: Vec<ChatCompletionRequestMessage> = context
            .into_iter()
            .map(|model_msg| model_msg.into())
            .collect();
        dual_debug!(
            "🔍 Model context - request_id: {}\n{}",
            request_id,
            serde_json::to_string_pretty(&context).unwrap()
        );
    }

    res
}

pub(crate) async fn embeddings_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(request): Json<EmbeddingRequest>,
) -> ServerResult<axum::response::Response> {
    // Get request ID from headers
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Received a new embeddings request - request_id: {}",
        request_id
    );

    // get the embeddings server
    let servers = state.server_group.read().await;
    let embeddings_servers = match servers.get(&ServerKind::embeddings) {
        Some(servers) => servers,
        None => {
            let err_msg = "No embedding server available. Please register a embedding server via the `/admin/servers/register` endpoint.";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    let embedding_server = match embeddings_servers.next().await {
        Ok(target_server_info) => target_server_info,
        Err(e) => {
            let err_msg = format!("Failed to get the embeddings server: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg));
        }
    };
    let embeddings_service_url =
        format!("{}/embeddings", embedding_server.url.trim_end_matches('/'));
    dual_info!(
        "Forward the embeddings request to {} - request_id: {}",
        embeddings_service_url,
        request_id
    );

    // parse the content-type header
    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            let err_msg = "Missing Content-Type header".to_string();
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;
    let content_type = content_type.to_string();
    dual_debug!(
        "Request content type: {} - request_id: {}",
        content_type,
        request_id
    );

    // Create request client
    let ds_request = if let Some(api_key) = &embedding_server.api_key
        && !api_key.is_empty()
    {
        reqwest::Client::new()
            .post(embeddings_service_url)
            .header("Content-Type", content_type)
            .header(AUTHORIZATION, api_key)
            .json(&request)
    } else if headers.contains_key("authorization") {
        let authorization = headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        reqwest::Client::new()
            .post(embeddings_service_url)
            .header("Content-Type", content_type)
            .header("Authorization", authorization)
            .json(&request)
    } else {
        reqwest::Client::new()
            .post(embeddings_service_url)
            .header("Content-Type", content_type)
            .json(&request)
    };

    // Use select! to handle request cancellation
    let ds_response = select! {
        response = ds_request.send() => {
            response.map_err(|e| {
                let err_msg = format!(
                    "Failed to forward the request to the downstream server: {e}",
                );
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    let status = ds_response.status();

    // Handle response body reading with cancellation
    let bytes = select! {
        bytes = ds_response.bytes() => {
            bytes.map_err(|e| {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled while reading response";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    match Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(bytes))
    {
        Ok(response) => {
            dual_info!(
                "Embeddings request completed successfully - request_id: {}",
                request_id
            );
            Ok(response)
        }
        Err(e) => {
            let err_msg = format!("Failed to create the response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

pub(crate) async fn audio_transcriptions_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    req: axum::extract::Request<Body>,
) -> ServerResult<axum::response::Response> {
    // Get request ID from headers
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Received a new audio transcription request - request_id: {}",
        request_id
    );

    // get the transcribe server
    let transcription_server = {
        let servers = state.server_group.read().await;
        let transcribe_servers = match servers.get(&ServerKind::transcribe) {
            Some(servers) => servers,
            None => {
                let err_msg = "No transcribe server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        };

        match transcribe_servers.next().await {
            Ok(target_server_info) => target_server_info,
            Err(e) => {
                let err_msg = format!("Failed to get the transcribe server: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg));
            }
        }
    };

    let transcription_server_url = format!(
        "{}/audio/transcriptions",
        transcription_server.url.trim_end_matches('/')
    );
    dual_info!(
        "Forward the audio transcription request to {} - request_id: {}",
        transcription_server_url,
        request_id
    );

    // Create request client
    let mut ds_request = reqwest::Client::new().post(transcription_server_url);
    if let Some(api_key) = &transcription_server.api_key
        && !api_key.is_empty()
    {
        ds_request = ds_request.header(AUTHORIZATION, api_key);
    }
    for (name, value) in req.headers().iter() {
        ds_request = ds_request.header(name, value);
    }

    // convert the request body into bytes
    let body = req.into_body();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| {
        let err_msg = format!("Failed to convert the request body into bytes: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        ServerError::Operation(err_msg)
    })?;

    ds_request = ds_request.body(body_bytes);

    // Use select! to handle request cancellation
    let ds_response = select! {
        response = ds_request.send() => {
            response.map_err(|e| {
                let err_msg = format!(
                    "Failed to forward the request to the downstream server: {e}"
                );
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    let status = ds_response.status();

    // Handle response body reading with cancellation
    let bytes = select! {
        bytes = ds_response.bytes() => {
            bytes.map_err(|e| {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled while reading response";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    match Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(bytes))
    {
        Ok(response) => {
            dual_info!(
                "Audio transcription request completed successfully - request_id: {}",
                request_id
            );
            Ok(response)
        }
        Err(e) => {
            let err_msg = format!("Failed to create the response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

pub(crate) async fn audio_translations_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    req: axum::extract::Request<Body>,
) -> ServerResult<axum::response::Response> {
    // Get request ID from headers
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Received a new audio translation request - request_id: {}",
        request_id
    );

    // get the transcribe server
    let translation_server = {
        let servers = state.server_group.read().await;
        let translate_servers = match servers.get(&ServerKind::translate) {
            Some(servers) => servers,
            None => {
                let err_msg = "No translate server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        };

        match translate_servers.next().await {
            Ok(target_server_info) => target_server_info,
            Err(e) => {
                let err_msg = format!("Failed to get the translate server: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg));
            }
        }
    };

    let translation_server_url = format!(
        "{}/audio/translations",
        translation_server.url.trim_end_matches('/')
    );
    dual_info!(
        "Forward the audio translation request to {} - request_id: {}",
        translation_server_url,
        request_id
    );

    // Create request client
    let mut ds_request = reqwest::Client::new().post(translation_server_url);
    if let Some(api_key) = &translation_server.api_key
        && !api_key.is_empty()
    {
        ds_request = ds_request.header(AUTHORIZATION, api_key);
    }
    for (name, value) in req.headers().iter() {
        ds_request = ds_request.header(name, value);
    }

    // convert the request body into bytes
    let body = req.into_body();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| {
        let err_msg = format!("Failed to convert the request body into bytes: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        ServerError::Operation(err_msg)
    })?;

    ds_request = ds_request.body(body_bytes);

    // Use select! to handle request cancellation
    let ds_response = select! {
        response = ds_request.send() => {
            response.map_err(|e| {
                let err_msg = format!(
                    "Failed to forward the request to the downstream server: {e}"
                );
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    let status = ds_response.status();

    // Handle response body reading with cancellation
    let bytes = select! {
        bytes = ds_response.bytes() => {
            bytes.map_err(|e| {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled while reading response";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    match Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .body(Body::from(bytes))
    {
        Ok(response) => {
            dual_info!(
                "Audio translation request completed successfully - request_id: {}",
                request_id
            );
            Ok(response)
        }
        Err(e) => {
            let err_msg = format!("Failed to create the response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

pub(crate) async fn audio_tts_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    req: axum::extract::Request<Body>,
) -> ServerResult<axum::response::Response> {
    // Get request ID from headers
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Received a new audio speech request - request_id: {}",
        request_id
    );

    // get the tts server
    let tts_server = {
        let servers = state.server_group.read().await;
        let tts_servers = match servers.get(&ServerKind::tts) {
            Some(servers) => servers,
            None => {
                let err_msg = "No tts server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        };

        match tts_servers.next().await {
            Ok(target_server_info) => target_server_info,
            Err(e) => {
                let err_msg = format!("Failed to get the tts server: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg));
            }
        }
    };

    let tts_server_url = format!("{}/audio/speech", tts_server.url.trim_end_matches('/'));
    dual_info!(
        "Forward the audio speech request to {} - request_id: {}",
        tts_server_url,
        request_id
    );

    // Create request client
    let mut ds_request = reqwest::Client::new().post(tts_server_url);
    if let Some(api_key) = &tts_server.api_key
        && !api_key.is_empty()
    {
        ds_request = ds_request.header(AUTHORIZATION, api_key);
    }
    for (name, value) in req.headers().iter() {
        ds_request = ds_request.header(name, value);
    }

    let body = req.into_body();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| {
        let err_msg = format!("Failed to convert the request body into bytes: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        ServerError::Operation(err_msg)
    })?;

    ds_request = ds_request.body(body_bytes);

    // Use select! to handle request cancellation
    let ds_response = select! {
        response = ds_request.send() => {
            response.map_err(|e| {
                let err_msg = format!(
                    "Failed to forward the request to the downstream server: {e}"
                );
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    // create a response builder with the status and headers of the downstream response
    let mut response_builder = Response::builder().status(ds_response.status());
    for (name, value) in ds_response.headers().iter() {
        response_builder = response_builder.header(name, value);
    }

    // Handle response body reading with cancellation
    let bytes = select! {
        bytes = ds_response.bytes() => {
            bytes.map_err(|e| {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled while reading response";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    match response_builder.body(Body::from(bytes)) {
        Ok(response) => {
            dual_info!(
                "Audio speech request completed successfully - request_id: {}",
                request_id
            );
            Ok(response)
        }
        Err(e) => {
            let err_msg = format!("Failed to create the response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

pub(crate) async fn image_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    req: axum::extract::Request<Body>,
) -> ServerResult<axum::response::Response> {
    // Get request ID from headers
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!("Received a new image request - request_id: {}", request_id);

    // get the image server
    let image_server = {
        let servers = state.server_group.read().await;
        let image_servers = match servers.get(&ServerKind::image) {
            Some(servers) => servers,
            None => {
                let err_msg = "No image server available";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        };

        match image_servers.next().await {
            Ok(target_server_info) => target_server_info,
            Err(e) => {
                let err_msg = format!("Failed to get the image server: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg));
            }
        }
    };

    let image_server_url = format!(
        "{}/images/generations",
        image_server.url.trim_end_matches('/')
    );
    dual_info!(
        "Forward the image request to {} - request_id: {}",
        image_server_url,
        request_id
    );

    // Create request client
    let mut ds_request = reqwest::Client::new().post(image_server_url);
    if let Some(api_key) = &image_server.api_key
        && !api_key.is_empty()
    {
        ds_request = ds_request.header(AUTHORIZATION, api_key);
    }
    for (name, value) in req.headers().iter() {
        ds_request = ds_request.header(name, value);
    }

    // convert the request body into bytes
    let body = req.into_body();
    let body_bytes = axum::body::to_bytes(body, usize::MAX).await.map_err(|e| {
        let err_msg = format!("Failed to convert the request body into bytes: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        ServerError::Operation(err_msg)
    })?;

    ds_request = ds_request.body(body_bytes);

    // Use select! to handle request cancellation
    let ds_response = select! {
        response = ds_request.send() => {
            response.map_err(|e| {
                let err_msg = format!(
                    "Failed to forward the request to the downstream server: {e}"
                );
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled by client";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    // create a response builder with the status and headers of the downstream response
    let mut response_builder = Response::builder().status(ds_response.status());
    for (name, value) in ds_response.headers().iter() {
        response_builder = response_builder.header(name, value);
    }

    // Handle response body reading with cancellation
    let bytes = select! {
        bytes = ds_response.bytes() => {
            bytes.map_err(|e| {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled while reading response";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            return Err(ServerError::Operation(warn_msg.to_string()));
        }
    };

    match response_builder.body(Body::from(bytes)) {
        Ok(response) => {
            dual_info!(
                "Image request completed successfully - request_id: {}",
                request_id
            );
            Ok(response)
        }
        Err(e) => {
            let err_msg = format!("Failed to create the response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            Err(ServerError::Operation(err_msg))
        }
    }
}

pub(crate) async fn models_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let models = state.models.read().await;
    let list_response = ListModelsResponse {
        object: String::from("list"),
        data: models.values().flatten().cloned().collect(),
    };

    let json_body = serde_json::to_string(&list_response).map_err(|e| {
        let err_msg = format!("Failed to serialize the models: {e}");
        dual_error!("{err_msg} - request_id: {request_id}");
        ServerError::Operation(err_msg)
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json_body))
        .map_err(|e| {
            let err_msg = format!("Failed to create response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            ServerError::Operation(err_msg)
        })
}

pub(crate) async fn info_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    let mut chat_models = vec![];
    let mut embedding_models = vec![];
    let mut image_models = vec![];
    let mut tts_models = vec![];
    let mut translate_models = vec![];
    let mut transcribe_models = vec![];
    let server_info = state.server_info.read().await;
    for server in server_info.servers.values() {
        if let Some(ref model) = server.chat_model {
            chat_models.push(model.clone());
        }
        if let Some(ref model) = server.embedding_model {
            embedding_models.push(model.clone());
        }
        if let Some(ref model) = server.image_model {
            image_models.push(model.clone());
        }
        if let Some(ref model) = server.tts_model {
            tts_models.push(model.clone());
        }
        if let Some(ref model) = server.translate_model {
            translate_models.push(model.clone());
        }
        if let Some(ref model) = server.transcribe_model {
            transcribe_models.push(model.clone());
        }
    }

    let json_body = serde_json::json!({
        "models": {
            "chat": chat_models,
            "embedding": embedding_models,
            "image": image_models,
            "tts": tts_models,
            "translate": translate_models,
            "transcribe": transcribe_models,
        },
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/json")
        .body(Body::from(json_body.to_string()))
        .map_err(|e| {
            let err_msg = format!("Failed to create response: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            ServerError::Operation(err_msg)
        })
}

/// Handler to get chat history by conversation ID
pub(crate) async fn get_conversation_history_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(conv_id): axum::extract::Path<String>,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Getting conversation history for conv_id: {} - request_id: {}",
        conv_id,
        request_id
    );

    if let Some(memory) = &state.memory {
        match memory.get_full_history(&conv_id, true).await {
            Ok(messages) => {
                dual_info!(
                    "Retrieved {} messages for conversation {} - request_id: {}",
                    messages.len(),
                    conv_id,
                    request_id
                );

                let response = serde_json::json!({
                    "conversation_id": conv_id,
                    "messages": messages
                });

                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(response.to_string()))
                    .map_err(|e| {
                        let err_msg = format!("Failed to create response: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })
            }
            Err(e) => {
                dual_error!(
                    "Failed to get conversation history for {}: {} - request_id: {}",
                    conv_id,
                    e,
                    request_id
                );
                Response::builder()
                    .status(StatusCode::NOT_FOUND)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "error": format!("Conversation not found: {}", e)
                        })
                        .to_string(),
                    ))
                    .map_err(|e| {
                        let err_msg = format!("Failed to create error response: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })
            }
        }
    } else {
        dual_warn!("Memory system is not enabled - request_id: {}", request_id);
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "error": "Memory system is not enabled"
                })
                .to_string(),
            ))
            .map_err(|e| {
                let err_msg = format!("Failed to create error response: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })
    }
}

/// Handler to get chat history by user ID
pub(crate) async fn get_user_history_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Getting user history for user_id: {} - request_id: {}",
        user_id,
        request_id
    );

    if let Some(memory) = &state.memory {
        match memory.get_user_full_history(&user_id, true).await {
            Ok(messages) => {
                dual_info!(
                    "Retrieved {} messages for user {} - request_id: {}",
                    messages.len(),
                    user_id,
                    request_id
                );

                let response = serde_json::json!({
                    "user_id": user_id,
                    "messages": messages
                });

                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(response.to_string()))
                    .map_err(|e| {
                        let err_msg = format!("Failed to create response: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })
            }
            Err(e) => {
                dual_error!(
                    "Failed to get user history for {}: {} - request_id: {}",
                    user_id,
                    e,
                    request_id
                );
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "error": format!("Failed to retrieve user history: {}", e)
                        })
                        .to_string(),
                    ))
                    .map_err(|e| {
                        let err_msg = format!("Failed to create error response: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })
            }
        }
    } else {
        dual_warn!("Memory system is not enabled - request_id: {}", request_id);
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "error": "Memory system is not enabled"
                })
                .to_string(),
            ))
            .map_err(|e| {
                let err_msg = format!("Failed to create error response: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })
    }
}

/// Handler to list conversations for a specific user
pub(crate) async fn list_user_conversations_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::extract::Path(user_id): axum::extract::Path<String>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> ServerResult<axum::response::Response> {
    let request_id = headers
        .get("x-request-id")
        .and_then(|h| h.to_str().ok())
        .unwrap_or("unknown")
        .to_string();

    dual_info!(
        "Listing conversations for user_id: {} - request_id: {}",
        user_id,
        request_id
    );

    // Parse limit parameter
    let limit = params.get("limit").and_then(|s| s.parse::<usize>().ok());

    if let Some(memory) = &state.memory {
        match memory.list_user_conversations(&user_id, limit).await {
            Ok(conversations) => {
                dual_info!(
                    "Retrieved {} conversations for user {} - request_id: {}",
                    conversations.len(),
                    user_id,
                    request_id
                );

                let response = serde_json::json!({
                    "user_id": user_id,
                    "conversations": conversations,
                    "total": conversations.len()
                });

                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(response.to_string()))
                    .map_err(|e| {
                        let err_msg = format!("Failed to create response: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })
            }
            Err(e) => {
                dual_error!(
                    "Failed to list conversations for user {}: {} - request_id: {}",
                    user_id,
                    e,
                    request_id
                );
                Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .header(CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "error": format!("Failed to retrieve user conversations: {}", e)
                        })
                        .to_string(),
                    ))
                    .map_err(|e| {
                        let err_msg = format!("Failed to create error response: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })
            }
        }
    } else {
        dual_warn!("Memory system is not enabled - request_id: {}", request_id);
        Response::builder()
            .status(StatusCode::SERVICE_UNAVAILABLE)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "error": "Memory system is not enabled"
                })
                .to_string(),
            ))
            .map_err(|e| {
                let err_msg = format!("Failed to create error response: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })
    }
}

pub(crate) mod admin {
    use super::*;

    pub(crate) async fn register_downstream_server_handler(
        State(state): State<Arc<AppState>>,
        headers: HeaderMap,
        Json(mut server): Json<Server>,
    ) -> ServerResult<axum::response::Response> {
        // Get request ID from headers
        let request_id = headers
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let server_url = server.url.clone();
        let server_kind = server.kind;
        let server_id = server.id.clone();

        // verify the server
        if server_kind.contains(ServerKind::chat)
            || server_kind.contains(ServerKind::embeddings)
            || server_kind.contains(ServerKind::image)
            || server_kind.contains(ServerKind::transcribe)
            || server_kind.contains(ServerKind::translate)
            || server_kind.contains(ServerKind::tts)
        {
            dual_warn!(
                "Ignore the server verification for: {server_id} - request_id: {request_id}"
            );
            // _verify_server(State(state.clone()), &headers, &request_id, &server).await?;
        }

        // update the model list
        update_model_list(State(state.clone()), &headers, &request_id, &server).await?;

        // update health status of the server
        server.health_status.is_healthy = true;
        server.health_status.last_check = SystemTime::now();

        // register the server
        state.register_downstream_server(server).await?;
        dual_info!(
            "Registered successfully. Assigned Server Id: {} - request_id: {}",
            server_id,
            request_id
        );

        // create a response with status code 200. Content-Type is JSON
        let json_body = serde_json::json!({
            "id": server_id,
            "url": server_url,
            "kind": server_kind
        });

        let response = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(json_body.to_string()))
            .map_err(|e| {
                let err_msg = format!("Failed to create response: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?;

        Ok(response)
    }

    // verify the server and get the server info and model list
    async fn _verify_server(
        State(state): State<Arc<AppState>>,
        headers: &HeaderMap,
        request_id: impl AsRef<str>,
        server: &Server,
    ) -> ServerResult<()> {
        let request_id = request_id.as_ref();
        let server_url = &server.url;
        let server_id = &server.id;
        let server_kind = server.kind;

        let server_info_url = format!("{server_url}/info");

        let client = reqwest::Client::new();
        let response = if let Some(api_key) = &server.api_key
            && !api_key.is_empty()
        {
            client
                .get(&server_info_url)
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, api_key)
                .send()
                .await
                .map_err(|e| {
                    let err_msg =
                        format!("Failed to verify the {server_kind} downstream server: {e}",);
                    dual_error!("{err_msg} - request_id: {request_id}");
                    ServerError::Operation(err_msg)
                })?
        } else if headers.contains_key("authorization") {
            let authorization = headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();

            client
                .get(&server_info_url)
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, authorization)
                .send()
                .await
                .map_err(|e| {
                    let err_msg =
                        format!("Failed to verify the {server_kind} downstream server: {e}",);
                    dual_error!("{err_msg} - request_id: {request_id}");
                    ServerError::Operation(err_msg)
                })?
        } else {
            client.get(&server_info_url).send().await.map_err(|e| {
                let err_msg = format!("Failed to verify the {server_kind} downstream server: {e}",);
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?
        };
        if !response.status().is_success() {
            let err_msg = format!(
                "Failed to verify the {} downstream server: {}",
                server_kind,
                response.status()
            );
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg));
        }

        let mut api_server = response.json::<ApiServer>().await.map_err(|e| {
            let err_msg = format!("Failed to parse the server info: {e}");
            dual_error!("{err_msg} - request_id: {request_id}");
            ServerError::Operation(err_msg)
        })?;
        api_server.server_id = Some(server_id.to_string());

        dual_debug!("server kind: {}", server_kind.to_string());
        dual_debug!("api server: {:?}", api_server);

        // verify the server kind
        {
            if server_kind.contains(ServerKind::chat) && api_server.chat_model.is_none() {
                let err_msg = "You are trying to register a chat server. However, the server does not support `chat`. Please check the server kind.";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
            if server_kind.contains(ServerKind::embeddings) && api_server.embedding_model.is_none()
            {
                let err_msg = "You are trying to register an embedding server. However, the server does not support `embeddings`. Please check the server kind.";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
            if server_kind.contains(ServerKind::image) && api_server.image_model.is_none() {
                let err_msg = "You are trying to register an image server. However, the server does not support `image`. Please check the server kind.";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
            if server_kind.contains(ServerKind::tts) && api_server.tts_model.is_none() {
                let err_msg = "You are trying to register a TTS server. However, the server does not support `tts`. Please check the server kind.";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
            if server_kind.contains(ServerKind::translate) && api_server.translate_model.is_none() {
                let err_msg = "You are trying to register a translation server. However, the server does not support `translate`. Please check the server kind.";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
            if server_kind.contains(ServerKind::transcribe) && api_server.transcribe_model.is_none()
            {
                let err_msg = "You are trying to register a transcription server. However, the server does not support `transcribe`. Please check the server kind.";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        }

        // update the server info
        let server_info = &mut state.server_info.write().await;
        server_info
            .servers
            .insert(server_id.to_string(), api_server);

        Ok(())
    }

    // update the model list
    pub(crate) async fn update_model_list(
        State(state): State<Arc<AppState>>,
        headers: &HeaderMap,
        request_id: impl AsRef<str>,
        server: &Server,
    ) -> ServerResult<()> {
        let request_id = request_id.as_ref();
        let server_url = &server.url;
        let server_id = &server.id;

        // get the models from the downstream server
        let list_models_url = format!("{server_url}/models");
        dual_debug!("list_models_url: {}", list_models_url);
        let response = if let Some(api_key) = &server.api_key
            && !api_key.is_empty()
        {
            reqwest::Client::new()
                .get(&list_models_url)
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, api_key)
                .send()
                .await
                .map_err(|e| {
                    let err_msg =
                        format!("Failed to get the models from the downstream server: {e}");
                    dual_error!("{err_msg} - request_id: {request_id}");
                    ServerError::Operation(err_msg)
                })?
        } else if headers.contains_key("authorization") {
            let authorization = headers
                .get("authorization")
                .unwrap()
                .to_str()
                .unwrap()
                .to_string();
            reqwest::Client::new()
                .get(&list_models_url)
                .header(CONTENT_TYPE, "application/json")
                .header(AUTHORIZATION, authorization)
                .send()
                .await
                .map_err(|e| {
                    let err_msg =
                        format!("Failed to get the models from the downstream server: {e}");
                    dual_error!("{err_msg} - request_id: {request_id}");
                    ServerError::Operation(err_msg)
                })?
        } else {
            reqwest::Client::new()
                .get(&list_models_url)
                .send()
                .await
                .map_err(|e| {
                    let err_msg =
                        format!("Failed to get the models from the downstream server: {e}");
                    dual_error!("{err_msg} - request_id: {request_id}");
                    ServerError::Operation(err_msg)
                })?
        };
        let status = response.status();
        if !status.is_success() {
            let err_msg =
                format!("Status: {status}. Failed to get model info from {list_models_url}.",);
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg));
        }

        match server_url.as_str() {
            "https://openrouter.ai/api/v1" => {
                let list_models_response =
                    response.json::<serde_json::Value>().await.map_err(|e| {
                        let err_msg =
                            format!("Failed to get the models from {list_models_url}: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })?;

                match list_models_response.get("data") {
                    Some(data) => {
                        // get `id` field from each model
                        let models = data.as_array().unwrap();
                        let model_info_vec = models
                            .iter()
                            .map(|model| {
                                let id = model.get("id").unwrap().as_str().unwrap();
                                let created = model.get("created").unwrap().as_u64().unwrap();
                                Model {
                                    id: id.to_string(),
                                    created,
                                    object: "model".to_string(),
                                    owned_by: "openrouter.ai".to_string(),
                                }
                            })
                            .collect::<Vec<Model>>();

                        // update the models
                        let mut models = state.models.write().await;
                        models.insert(server_id.to_string(), model_info_vec);
                    }
                    None => {
                        let err_msg = format!(
                            "Failed to get the models from {list_models_url}. Not found `data` field in the response."
                        );
                        dual_error!("{err_msg} - request_id: {request_id}");
                        return Err(ServerError::Operation(err_msg.to_string()));
                    }
                }
            }
            _ => {
                let list_models_response =
                    response.json::<ListModelsResponse>().await.map_err(|e| {
                        let err_msg =
                            format!("Failed to get the models from {list_models_url}: {e}");
                        dual_error!("{err_msg} - request_id: {request_id}");
                        ServerError::Operation(err_msg)
                    })?;

                // update the models
                let mut models = state.models.write().await;
                models.insert(server_id.to_string(), list_models_response.data);
            }
        }

        Ok(())
    }

    pub(crate) async fn remove_downstream_server_handler(
        State(state): State<Arc<AppState>>,
        headers: HeaderMap,
        Json(server_id): Json<ServerIdToRemove>,
    ) -> ServerResult<axum::response::Response> {
        // Get request ID from headers
        let request_id = headers
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        state
            .unregister_downstream_server(&server_id.server_id)
            .await?;

        // create a response with status code 200. Content-Type is JSON
        let json_body = serde_json::json!({
            "message": "Server unregistered successfully.",
            "id": server_id.server_id,
        });

        let response = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(json_body.to_string()))
            .map_err(|e| {
                let err_msg = format!("Failed to create response: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?;

        Ok(response)
    }

    pub(crate) async fn list_downstream_servers_handler(
        State(state): State<Arc<AppState>>,
        headers: HeaderMap,
    ) -> ServerResult<axum::response::Response> {
        // Get request ID from headers
        let request_id = headers
            .get("x-request-id")
            .and_then(|h| h.to_str().ok())
            .unwrap_or("unknown")
            .to_string();

        let servers = state.list_downstream_servers().await?;

        // compute the total number of servers
        let total_servers = servers.values().fold(0, |acc, servers| acc + servers.len());
        dual_info!(
            "Found {} downstream servers - request_id: {}",
            total_servers,
            request_id
        );

        let json_body = serde_json::to_string(&servers).unwrap();

        let response = Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", "application/json")
            .body(Body::from(json_body))
            .map_err(|e| {
                let err_msg = format!("Failed to create response: {e}");
                dual_error!("{err_msg} - request_id: {request_id}");
                ServerError::Operation(err_msg)
            })?;

        Ok(response)
    }
}
