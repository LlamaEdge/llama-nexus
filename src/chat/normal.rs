use std::{sync::Arc, time::SystemTime};

use axum::{
    Json,
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, Response, StatusCode},
};
use bytes::Bytes;
use endpoints::{
    chat::{
        ChatCompletionAssistantMessage, ChatCompletionChunk, ChatCompletionChunkChoice,
        ChatCompletionChunkChoiceDelta, ChatCompletionObject, ChatCompletionRequest,
        ChatCompletionRequestMessage, ChatCompletionRole, ChatCompletionToolMessage,
        ChatCompletionUserMessageContent, Function, ToolCall, ToolChoice,
    },
    common::FinishReason,
};
use futures_util::{StreamExt, stream};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rmcp::model::{CallToolRequestParam, RawContent};
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::{
    AppState,
    chat::{gen_chat_id, utils::*},
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{DEFAULT_SEARCH_FALLBACK_MESSAGE, MCP_SEPARATOR, MCP_SERVICES, SEARCH_MCP_SERVER_NAMES},
    memory::{ModelRole, ModelToolCall, StoredToolCall},
    server::{RoutingPolicy, ServerKind, TargetServerInfo},
};

pub(crate) async fn chat(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
    conv_id: Option<String>,
    request_id: impl AsRef<str>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();

    // Extract user message for memory storage
    let user_message = extract_user_message(&request);

    // Extract system message for memory storage
    let system_message = extract_system_message(&request);

    // Get target server
    let chat_server = get_chat_server(&state, request_id).await?;

    // Store the latest user message to memory
    if let Some(memory) = &state.memory
        && let Some(conv_id) = &conv_id
        && let Some(user_msg) = &user_message
    {
        // First handle system message storage (if exists)
        if let Some(sys_msg) = &system_message {
            match memory.set_system_message(conv_id, sys_msg).await {
                Ok(updated) => {
                    if updated {
                        dual_debug!(
                            "System message updated for conversation {} - request_id: {}",
                            conv_id,
                            request_id
                        );
                    }
                }
                Err(e) => {
                    dual_error!(
                        "Failed to store system message in memory: {} - request_id: {}",
                        e,
                        request_id
                    );
                }
            }
        }

        // Store user message to memory
        match memory.add_user_message(conv_id, user_msg.clone()).await {
            Ok(_) => {
                dual_debug!(
                    "🔍 User message added to memory for conversation {} - request_id: {}",
                    conv_id,
                    request_id
                );

                // get model context
                let context = memory.get_model_context(conv_id).await.map_err(|e| {
                    let err_msg = format!("Failed to get model context: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;
                let context: Vec<ChatCompletionRequestMessage> = context
                    .into_iter()
                    .map(|model_msg| model_msg.into())
                    .collect();

                // update request.messages with model context
                request.messages = context;

                dual_debug!(
                    "🔍 Request messages updated with model context - request_id: {}\n{}",
                    request_id,
                    serde_json::to_string_pretty(&request.messages).unwrap()
                );
            }
            Err(e) => {
                dual_error!(
                    "Failed to add user message to memory: {} - request_id: {}",
                    e,
                    request_id
                );
            }
        }
    }

    // set non-stream mode
    let stream = request.stream.unwrap_or(false);
    if stream {
        request.stream = Some(false);
    }

    // Build and send request
    let response = {
        let url = format!("{}/chat/completions", chat_server.url.trim_end_matches('/'));
        let mut client = reqwest::Client::new().post(&url);

        // Add common headers
        client = client.header(CONTENT_TYPE, "application/json");

        // Add authorization header
        if let Some(api_key) = &chat_server.api_key
            && !api_key.is_empty()
        {
            let auth_info = if api_key.starts_with("Bearer ") {
                api_key.clone()
            } else {
                format!("Bearer {api_key}")
            };

            dual_info!("auth_info: {}", &auth_info);

            client = client.header(AUTHORIZATION, auth_info);
        } else if let Some(auth) = headers.get("authorization")
            && let Ok(auth_str) = auth.to_str()
        {
            client = client.header(AUTHORIZATION, auth_str);
        }

        dual_info!(
            "Request to downstream chat server - request_id: {}\n{}",
            request_id,
            serde_json::to_string_pretty(&request).unwrap()
        );

        // Use select! to support cancellation
        let response = select! {
            response = client.json(&request).send() => {
                response.map_err(|e| ServerError::Operation(format!("Failed to forward request: {e}")))
            }
            _ = cancel_token.cancelled() => {
                let warn_msg = "Request was cancelled by client";
                dual_warn!("{}", warn_msg);
                Err(ServerError::Operation(warn_msg.to_string()))
            }
        };

        response?
    };

    // check the status code
    let status = response.status();
    let response_result = match status {
        StatusCode::OK => {
            let response_headers = response.headers().clone();

            // Read the response body
            let bytes = read_response_bytes(response, request_id, cancel_token.clone()).await?;
            let chat_completion = parse_chat_completion(&bytes, request_id)?;

            // Check if the response requires tool call
            let requires_tool_call = !chat_completion.choices[0].message.tool_calls.is_empty();
            if requires_tool_call {
                // Convert tool calls to stored format for memory
                let stored_tool_calls = if let Some(conv_id) = &conv_id {
                    Some(convert_tool_calls_to_stored(
                        &chat_completion.choices[0].message.tool_calls,
                        conv_id,
                    ))
                } else {
                    None
                };

                // TODO: to support multiple tool calls
                let tool_call = &chat_completion.choices[0].message.tool_calls[0];
                let contains = tool_call.function.name.as_str().contains(MCP_SEPARATOR);
                let parts: Vec<&str> = tool_call
                    .function
                    .name
                    .as_str()
                    .split(MCP_SEPARATOR)
                    .collect();
                if contains && parts.len() == 2 {
                    call_mcp_server(
                        State(state.clone()),
                        tool_call,
                        &mut request,
                        &headers,
                        stream,
                        &chat_server,
                        request_id,
                        cancel_token,
                        conv_id.as_deref(),
                        stored_tool_calls,
                    )
                    .await
                } else {
                    let err_msg = format!(
                        "The tool call '{}' is not supported.",
                        tool_call.function.name
                    );
                    dual_error!("{}", err_msg);
                    return Err(ServerError::Operation(err_msg));
                }
            } else {
                let assistant_msg = match &chat_completion.choices[0].message.content {
                    Some(content) if !content.is_empty() => content.clone(),
                    _ => String::new(),
                };

                // Store assistant message to memory
                if let Some(memory) = &state.memory
                    && let Some(conv_id) = &conv_id
                    && let Err(e) = memory
                        .add_assistant_message(conv_id, &assistant_msg, vec![])
                        .await
                {
                    dual_error!(
                        "Failed to add assistant message to memory: {} - request_id: {}",
                        e,
                        request_id
                    );
                }

                // Return chat completion
                match stream {
                    true => {
                        let chunks = gen_chunks_with_formatting(&assistant_msg, 10);
                        let id = match &request.user {
                            Some(id) => id.clone(),
                            None => gen_chat_id(),
                        };
                        let model = chat_completion.model.clone();
                        let usage = chat_completion.usage;
                        let chunks_len = chunks.len();

                        // Create SSE stream
                        let request_id_owned = request_id.to_string();
                        let stream =
                            stream::iter(chunks.into_iter().enumerate().map(move |(i, chunk)| {
                                let created = SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map_err(|e| {
                                        let err_msg =
                                            format!("Failed to get the current time. Reason: {e}");

                                        dual_error!(
                                            "{} - request_id: {}",
                                            err_msg,
                                            request_id_owned
                                        );

                                        ServerError::Operation(err_msg)
                                    })
                                    .unwrap();

                                let mut chat_completion_chunk = ChatCompletionChunk {
                                    id: id.clone(),
                                    object: "chat.completion.chunk".to_string(),
                                    created: created.as_secs(),
                                    model: model.clone(),
                                    system_fingerprint: "fp_44709d6fcb".to_string(),
                                    choices: vec![ChatCompletionChunkChoice {
                                        index: i as u32,
                                        delta: ChatCompletionChunkChoiceDelta {
                                            role: ChatCompletionRole::Assistant,
                                            content: Some(chunk),
                                            tool_calls: vec![],
                                        },
                                        logprobs: None,
                                        finish_reason: None,
                                    }],
                                    usage: None,
                                };

                                if i == chunks_len - 1 {
                                    // update finish_reason
                                    chat_completion_chunk.choices[0].finish_reason =
                                        Some(FinishReason::stop);

                                    // update usage
                                    chat_completion_chunk.usage = Some(usage);
                                }

                                let json_str =
                                    serde_json::to_string(&chat_completion_chunk).unwrap();
                                format!("data: {json_str}\n\n")
                            }))
                            .chain(stream::once(async { "data: [DONE]\n\n".to_string() }))
                            .map(|s| Ok::<_, std::convert::Infallible>(s.into_bytes()));

                        // Build streaming response
                        let response = Response::builder()
                            .header(CONTENT_TYPE, "text/event-stream")
                            .header("Cache-Control", "no-cache")
                            .header("Connection", "keep-alive")
                            .status(StatusCode::OK)
                            .body(Body::from_stream(stream));

                        match response {
                            Ok(response) => {
                                dual_info!(
                                    "Streaming response sent successfully - request_id: {}",
                                    request_id
                                );
                                return Ok(response);
                            }
                            Err(e) => {
                                let err_msg = format!("Failed to create streaming response: {e}");
                                dual_error!("{} - request_id: {}", err_msg, request_id);
                                return Err(ServerError::Operation(err_msg));
                            }
                        }
                    }
                    false => build_response(status, response_headers, bytes, request_id),
                }
            }
        }
        _ => {
            // Convert reqwest::Response to axum::Response
            let status = response.status();

            let err_msg = format!("{status}");
            dual_error!("{} - request_id: {}", err_msg, request_id);

            let headers = response.headers().clone();
            let bytes = response.bytes().await.map_err(|e| {
                let err_msg = format!("Failed to get response bytes: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                ServerError::Operation(err_msg)
            })?;

            build_response(status, headers, bytes, request_id)
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

    response_result
}

async fn get_chat_server(
    state: &Arc<AppState>,
    request_id: &str,
) -> ServerResult<crate::server::TargetServerInfo> {
    let servers = state.server_group.read().await;
    let chat_servers = match servers.get(&ServerKind::chat) {
        Some(servers) => servers,
        None => {
            let err_msg = "No chat server available. Please register a chat server via the `/admin/servers/register` endpoint.";
            dual_error!("{} - request_id: {}", err_msg, request_id);
            return Err(ServerError::Operation(err_msg.to_string()));
        }
    };

    match chat_servers.next().await {
        Ok(target_server_info) => Ok(target_server_info),
        Err(e) => {
            let err_msg = format!("Failed to get the chat server: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            Err(ServerError::Operation(err_msg))
        }
    }
}

/// Copy HTTP response headers to response builder
///
/// Selectively copy response headers based on whether it's a streaming response.
///
/// # Arguments
///
/// * `response_builder` - Response builder
/// * `headers` - Source response headers
fn copy_response_headers(
    response_builder: axum::http::response::Builder,
    headers: &HeaderMap,
) -> axum::http::response::Builder {
    let allowed_headers = [
        "access-control-allow-origin",
        "access-control-allow-headers",
        "access-control-allow-methods",
        "content-type",
        "content-length",
        "cache-control",
        "connection",
        "user",
        "date",
        "requires-tool-call",
    ];

    headers
        .iter()
        .fold(response_builder, |builder, (name, value)| {
            if allowed_headers.contains(&name.as_str()) {
                dual_debug!("copy header: {} - {}", name, value.to_str().unwrap());
                builder.header(name, value)
            } else {
                dual_debug!("ignore header: {} - {}", name, value.to_str().unwrap());
                builder
            }
        })
}

/// Read HTTP response body data with cancellation support
///
/// This function uses select! macro to simultaneously monitor response reading and cancellation signals.
/// When the request is cancelled, it immediately returns an error to avoid resource waste.
async fn read_response_bytes(
    response: reqwest::Response,
    request_id: &str,
    cancel_token: CancellationToken,
) -> ServerResult<Bytes> {
    select! {
        bytes = response.bytes() => {
            bytes.map_err(|e| {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                ServerError::Operation(err_msg)
            })
        }
        _ = cancel_token.cancelled() => {
            let warn_msg = "Request was cancelled while reading response";
            dual_warn!("{} - request_id: {}", warn_msg, request_id);
            Err(ServerError::Operation(warn_msg.to_string()))
        }
    }
}

fn parse_chat_completion(bytes: &Bytes, request_id: &str) -> ServerResult<ChatCompletionObject> {
    serde_json::from_slice(bytes).map_err(|e| {
        let value = serde_json::from_slice::<serde_json::Value>(bytes).unwrap();

        dual_error!(
            "The response body received from the downstream server - request_id: {}:\n{}",
            request_id,
            serde_json::to_string_pretty(&value).unwrap()
        );

        let err_msg = format!("Failed to parse the response: {e}");

        dual_error!("{} - request_id: {}", err_msg, request_id);

        ServerError::Operation(err_msg)
    })
}

/// Build HTTP response object
///
/// Build complete HTTP response based on status code, response headers and response body data.
/// Copy all response headers to the new response and log success message.
///
/// # Arguments
///
/// * `status` - HTTP status code
/// * `response_headers` - Response headers
/// * `bytes` - Response body data
/// * `request_id` - Request ID for logging
fn build_response(
    status: StatusCode,
    response_headers: HeaderMap,
    bytes: Bytes,
    request_id: &str,
) -> ServerResult<axum::response::Response> {
    // build the response builder
    let mut response_builder = Response::builder().status(status);

    // copy the response headers
    response_builder = copy_response_headers(response_builder, &response_headers);

    match response_builder.body(Body::from(bytes)) {
        Ok(response) => {
            dual_info!(
                "Chat request completed successfully - request_id: {}",
                request_id
            );

            Ok(response)
        }
        Err(e) => {
            let err_msg = format!("Failed to create the response: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            Err(ServerError::Operation(err_msg))
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn call_mcp_server(
    State(state): State<Arc<AppState>>,
    tool_call: &ToolCall,
    request: &mut ChatCompletionRequest,
    headers: &HeaderMap,
    stream: bool,
    chat_server: &TargetServerInfo,
    request_id: impl AsRef<str>,
    cancel_token: CancellationToken,
    conv_id: Option<&str>,
    mut stored_tool_calls: Option<Vec<StoredToolCall>>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();
    let chat_service_url = format!("{}/chat/completions", chat_server.url.trim_end_matches('/'));

    let parts: Vec<&str> = tool_call
        .function
        .name
        .as_str()
        .split(MCP_SEPARATOR)
        .collect();
    let mcp_tool_name = parts[0];
    let mcp_server_name = parts[1];
    let mcp_tool_args = tool_call.function.arguments.as_str();
    let tool_call_id = tool_call.id.as_str();

    dual_info!(
        "Mcp server: {}, tool: {}, Tool args: {} - request_id: {}",
        mcp_server_name,
        mcp_tool_name,
        mcp_tool_args,
        request_id
    );

    if let Some(services) = MCP_SERVICES.get() {
        let service_map = services.read().await;
        // get the mcp client
        let service = match service_map.get(mcp_server_name) {
            Some(mcp_client) => mcp_client,
            None => {
                let err_msg =
                    format!("Not found mcp client connected with {mcp_server_name} mcp server");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::McpOperation(err_msg.to_string()));
            }
        };

        // call a tool
        let request_param = CallToolRequestParam {
            name: mcp_tool_name.to_string().into(),
            arguments: serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(
                mcp_tool_args,
            )
            .ok(),
        };
        let tool_result = service
            .read()
            .await
            .raw
            .call_tool(request_param)
            .await
            .map_err(|e| {
                dual_error!("Failed to call the mcp tool. {}", e);
                ServerError::Operation(e.to_string())
            })?;
        dual_debug!("{}", serde_json::to_string_pretty(&tool_result).unwrap());

        match tool_result.is_error {
            Some(false) => {
                match &tool_result.content {
                    Some(content) => {
                        let content = &content[0];
                        match &content.raw {
                            RawContent::Text(text) => {
                                dual_info!(
                                    "The tool call result returned by {} mcp server: {:#?}",
                                    &mcp_server_name,
                                    text.text
                                );

                                let content = match SEARCH_MCP_SERVER_NAMES
                                    .contains(&mcp_server_name)
                                {
                                    true => {
                                        // get the fallback message from the mcp client
                                        let fallback = if service
                                            .read()
                                            .await
                                            .has_fallback_message()
                                        {
                                            service.read().await.fallback_message.clone().unwrap()
                                        } else {
                                            DEFAULT_SEARCH_FALLBACK_MESSAGE.to_string()
                                        };

                                        dual_debug!(
                                            "fallback message: {} - request_id: {}",
                                            fallback,
                                            request_id
                                        );

                                        // add tool results as context
                                        let content = format!(
                                            "Please answer the question based on the information between **---BEGIN CONTEXT---** and **---END CONTEXT---**. Do not use any external knowledge. If the information between **---BEGIN CONTEXT---** and **---END CONTEXT---** is empty, please respond with `{fallback}`. Note that DO NOT use any tools if provided.\n\n---BEGIN CONTEXT---\n\n{context}\n\n---END CONTEXT---",
                                            fallback = fallback,
                                            context = &text.text,
                                        );

                                        content
                                    }
                                    false => text.text.clone(),
                                };

                                dual_debug!("context:\n{}", &content);

                                // Store tool calls and results to memory
                                if let (Some(conv_id), Some(stored_tcs), Some(memory)) =
                                    (conv_id, stored_tool_calls.as_mut(), &state.memory)
                                {
                                    // Add tool results to stored tool calls
                                    add_tool_results_to_stored(
                                        stored_tcs,
                                        std::slice::from_ref(&content),
                                    );

                                    if let Err(e) = memory
                                        .add_assistant_message(conv_id, "", stored_tcs.clone())
                                        .await
                                    {
                                        dual_error!(
                                            "Failed to store tool calls to memory: {} - request_id: {}",
                                            e,
                                            request_id
                                        );
                                    }
                                }

                                // update request messages
                                if let (Some(conv_id), Some(memory)) = (conv_id, &state.memory) {
                                    let context =
                                        memory.get_model_context(conv_id).await.map_err(|e| {
                                            let err_msg =
                                                format!("Failed to get model context: {e}");
                                            dual_error!("{} - request_id: {}", err_msg, request_id);
                                            ServerError::Operation(err_msg)
                                        })?;
                                    let context: Vec<ChatCompletionRequestMessage> = context
                                        .into_iter()
                                        .map(|model_msg| model_msg.into())
                                        .collect();

                                    // Update request messages with context
                                    request.messages = context;
                                } else {
                                    // append assistant message with tool call to request messages
                                    let assistant_completion_message =
                                        ChatCompletionRequestMessage::Assistant(
                                            ChatCompletionAssistantMessage::new(
                                                None,
                                                None,
                                                Some(vec![tool_call.clone()]),
                                            ),
                                        );
                                    request.messages.push(assistant_completion_message);

                                    // append tool message with tool result to request messages
                                    let tool_completion_message =
                                        ChatCompletionRequestMessage::Tool(
                                            ChatCompletionToolMessage::new(&content, tool_call_id),
                                        );
                                    request.messages.push(tool_completion_message);
                                }

                                // disable tool choice
                                if request.tool_choice.is_some() {
                                    request.tool_choice = Some(ToolChoice::None);
                                }

                                // Create a request client that can be cancelled
                                let ds_request = if let Some(api_key) = &chat_server.api_key
                                    && !api_key.is_empty()
                                {
                                    let auth_info = if api_key.starts_with("Bearer ") {
                                        api_key.clone()
                                    } else {
                                        format!("Bearer {api_key}")
                                    };

                                    reqwest::Client::new()
                                        .post(&chat_service_url)
                                        .header(CONTENT_TYPE, "application/json")
                                        .header(AUTHORIZATION, auth_info)
                                        .json(&request)
                                } else if headers.contains_key("authorization") {
                                    let authorization = headers
                                        .get("authorization")
                                        .unwrap()
                                        .to_str()
                                        .unwrap()
                                        .to_string();

                                    reqwest::Client::new()
                                        .post(&chat_service_url)
                                        .header(CONTENT_TYPE, "application/json")
                                        .header(AUTHORIZATION, authorization)
                                        .json(&request)
                                } else {
                                    reqwest::Client::new()
                                        .post(&chat_service_url)
                                        .header(CONTENT_TYPE, "application/json")
                                        .json(&request)
                                };

                                dual_debug!(
                                    "Request to downstream chat server - request_id: {}\n{}",
                                    request_id,
                                    serde_json::to_string_pretty(&request).unwrap()
                                );

                                // Use select! to handle request cancellation
                                let ds_response = select! {
                                    response = ds_request.send() => {
                                        response.map_err(|e| {
                                            let err_msg = format!(
                                                "Failed to forward the request to the downstream server: {e}"
                                            );
                                            dual_error!("{} - request_id: {}", err_msg, request_id);
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
                                match status {
                                    StatusCode::OK => {
                                        let mut response_builder =
                                            Response::builder().status(status);

                                        // copy the response headers
                                        let headers = ds_response.headers().clone();

                                        // Handle response body reading with cancellation
                                        let bytes = select! {
                                            bytes = ds_response.bytes() => {
                                                bytes.map_err(|e| {
                                                    let err_msg = format!("Failed to get the full response as bytes: {e}");
                                                    dual_error!("{} - request_id: {}", err_msg, request_id);
                                                    ServerError::Operation(err_msg)
                                                })?
                                            }
                                            _ = cancel_token.cancelled() => {
                                                let warn_msg = "Request was cancelled while reading response";
                                                dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                return Err(ServerError::Operation(warn_msg.to_string()));
                                            }
                                        };

                                        let chat_completion =
                                            parse_chat_completion(&bytes, request_id)?;

                                        let assistant_message = chat_completion
                                            .choices
                                            .first()
                                            .and_then(|choice| choice.message.content.clone())
                                            .unwrap_or_default();

                                        // Store final assistant message to memory
                                        if let (Some(conv_id), Some(memory)) =
                                            (conv_id, &state.memory)
                                            && let Err(e) = memory
                                                .add_assistant_message(
                                                    conv_id,
                                                    &assistant_message,
                                                    vec![],
                                                )
                                                .await
                                        {
                                            dual_warn!(
                                                "Failed to add assistant message to memory: {e} - request_id: {}",
                                                request_id
                                            );
                                        }

                                        // Return final response
                                        match stream {
                                            true => {
                                                let chunks = gen_chunks_with_formatting(
                                                    &assistant_message,
                                                    10,
                                                );
                                                let id = match &request.user {
                                                    Some(id) => id.clone(),
                                                    None => gen_chat_id(),
                                                };
                                                let model = chat_completion.model.clone();
                                                let usage = chat_completion.usage;
                                                let chunks_len = chunks.len();

                                                // Create SSE stream
                                                let request_id_owned = request_id.to_string();
                                                let stream = stream::iter(chunks.into_iter().enumerate().map(
                                                    move |(i, chunk)| {
                                                        let created = SystemTime::now()
                                                            .duration_since(std::time::UNIX_EPOCH)
                                                            .map_err(|e| {
                                                                let err_msg = format!(
                                                                    "Failed to get the current time. Reason: {e}"
                                                                );

                                                                dual_error!(
                                                                    "{} - request_id: {}",
                                                                    err_msg,
                                                                    request_id_owned
                                                                );

                                                                ServerError::Operation(err_msg)
                                                            })
                                                            .unwrap();

                                                        let mut chat_completion_chunk = ChatCompletionChunk {
                                                            id: id.clone(),
                                                            object: "chat.completion.chunk".to_string(),
                                                            created: created.as_secs(),
                                                            model: model.clone(),
                                                            system_fingerprint: "fp_44709d6fcb".to_string(),
                                                            choices: vec![ChatCompletionChunkChoice {
                                                                index: i as u32,
                                                                delta: ChatCompletionChunkChoiceDelta {
                                                                    role: ChatCompletionRole::Assistant,
                                                                    content: Some(chunk),
                                                                    tool_calls: vec![],
                                                                },
                                                                logprobs: None,
                                                                finish_reason: None,
                                                            }],
                                                            usage: None,
                                                        };

                                                        if i == chunks_len - 1 {
                                                            // update finish_reason
                                                            chat_completion_chunk.choices[0].finish_reason =
                                                                Some(FinishReason::stop);

                                                            // update usage
                                                            chat_completion_chunk.usage = Some(usage);
                                                        }

                                                        let json_str =
                                                            serde_json::to_string(&chat_completion_chunk).unwrap();
                                                        format!("data: {json_str}\n\n")
                                                    },
                                                ))
                                                .chain(stream::once(async { "data: [DONE]\n\n".to_string() }))
                                                .map(|s| Ok::<_, std::convert::Infallible>(s.into_bytes()));

                                                // Build streaming response
                                                Response::builder()
                                                    .header(
                                                        CONTENT_TYPE,
                                                        "text/event-stream",
                                                    )
                                                    .header(
                                                        "Cache-Control",
                                                        "no-cache",
                                                    )
                                                    .header(
                                                        "Connection",
                                                        "keep-alive",
                                                    )
                                                    .status(StatusCode::OK)
                                                    .body(Body::from_stream(
                                                        stream,
                                                    )).map_err(|e| {
                                                        let err_msg = format!(
                                                            "Failed to create streaming response: {e}"
                                                        );
                                                        dual_error!(
                                                            "{} - request_id: {}",
                                                            err_msg,
                                                            request_id
                                                        );
                                                        ServerError::Operation(err_msg)
                                                    })
                                            }
                                            false => {
                                                let response_body =
                                                    serde_json::to_string(&chat_completion)
                                                        .unwrap();

                                                response_builder = copy_response_headers(
                                                    response_builder,
                                                    &headers,
                                                );

                                                response_builder.body(Body::from(response_body)).map_err(|e| {
                                                    let err_msg = format!("Failed to create the response body: {e}");
                                                    dual_error!("{} - request_id: {}", err_msg, request_id);
                                                    ServerError::Operation(err_msg)
                                                })
                                            }
                                        }
                                    }
                                    _ => {
                                        // Convert reqwest::Response to axum::Response
                                        let status = ds_response.status();

                                        let err_msg = format!("{status}");
                                        dual_error!("{} - request_id: {}", err_msg, request_id);

                                        let headers = ds_response.headers().clone();
                                        let bytes = ds_response.bytes().await.map_err(|e| {
                                            let err_msg =
                                                format!("Failed to get response bytes: {e}");
                                            dual_error!("{} - request_id: {}", err_msg, request_id);
                                            ServerError::Operation(err_msg)
                                        })?;

                                        build_response(status, headers, bytes, request_id)
                                    }
                                }
                            }
                            _ => {
                                let err_msg =
                                    "Only text content is supported for tool call results";
                                dual_error!("{} - request_id: {}", err_msg, request_id);
                                Err(ServerError::Operation(err_msg.to_string()))
                            }
                        }
                    }
                    None => {
                        let err_msg = "The mcp tool result is empty";
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        Err(ServerError::McpEmptyContent)
                    }
                }
            }
            _ => {
                let err_msg = format!("Failed to call the tool: {mcp_tool_name}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                Err(ServerError::Operation(err_msg))
            }
        }
    } else {
        let err_msg = "Empty MCP CLIENTS";
        dual_error!("{} - request_id: {}", err_msg, request_id);
        Err(ServerError::McpOperation(err_msg.to_string()))
    }
}

impl From<crate::memory::types::ModelMessage> for ChatCompletionRequestMessage {
    fn from(msg: crate::memory::types::ModelMessage) -> Self {
        match msg.role {
            ModelRole::System => {
                ChatCompletionRequestMessage::new_system_message(&msg.content, None)
            }
            ModelRole::User => ChatCompletionRequestMessage::new_user_message(
                ChatCompletionUserMessageContent::Text(msg.content),
                None,
            ),
            ModelRole::Assistant => {
                match msg.tool_calls {
                    Some(tool_calls) => {
                        // Convert tool calls to the appropriate format
                        let tool_calls = tool_calls
                            .into_iter()
                            .map(|tool_call| tool_call.into())
                            .collect();

                        // Build the assistant message with tool calls
                        ChatCompletionRequestMessage::new_assistant_message(
                            Some(msg.content),
                            None,
                            Some(tool_calls),
                        )
                    }
                    None => {
                        // Otherwise, only include message content
                        ChatCompletionRequestMessage::new_assistant_message(
                            Some(msg.content),
                            None,
                            None,
                        )
                    }
                }
            }
            ModelRole::Tool => ChatCompletionRequestMessage::Tool(ChatCompletionToolMessage::new(
                &msg.content,
                msg.tool_call_id.as_deref().unwrap(),
            )),
        }
    }
}

impl From<ModelToolCall> for ToolCall {
    fn from(tool_call: ModelToolCall) -> Self {
        ToolCall {
            id: tool_call.id,
            ty: tool_call.ty,
            function: Function {
                name: tool_call.function.name,
                arguments: tool_call.function.arguments,
            },
        }
    }
}
