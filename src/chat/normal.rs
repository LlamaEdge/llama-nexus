use std::sync::Arc;

use axum::{
    Json,
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, Response, StatusCode},
};
use bytes::Bytes;
use endpoints::chat::{
    ChatCompletionAssistantMessage, ChatCompletionChunk, ChatCompletionObject,
    ChatCompletionRequest, ChatCompletionRequestMessage, ChatCompletionToolMessage,
    ChatCompletionUserMessageContent, Function, ToolCall, ToolChoice,
};
use futures_util::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rmcp::model::{CallToolRequestParam, RawContent};
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::{
    AppState, dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{DEFAULT_SEARCH_FALLBACK_MESSAGE, MCP_SERVICES, MCP_TOOLS, SEARCH_MCP_SERVER_NAMES},
    memory::{ModelRole, ModelToolCall, StoredToolCall, StoredToolResult},
    server::{RoutingPolicy, ServerKind, TargetServerInfo},
};

pub(crate) async fn chat(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
    request_id: impl AsRef<str>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();

    // Extract user message for memory storage
    let user_message = extract_user_message(&request);

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

    // Get target server
    let chat_server = get_chat_server(&state, request_id).await?;

    // 存储用户消息到记忆中
    if let Some(memory) = &state.memory
        && let Some(conv_id) = &conv_id
        && let Some(user_msg) = &user_message
        && let Err(e) = memory.add_user_message(conv_id, user_msg.clone()).await
    {
        dual_error!(
            "Failed to add user message to memory: {} - request_id: {}",
            e,
            request_id
        );
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
            client = client.header(AUTHORIZATION, api_key);
        } else if let Some(auth) = headers.get("authorization")
            && let Ok(auth_str) = auth.to_str()
        {
            client = client.header(AUTHORIZATION, auth_str);
        }

        dual_debug!(
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

    // Handle response based on stream mode
    let response_result = match request.stream {
        Some(true) => {
            let status = response.status();

            // check the status code
            match status {
                StatusCode::OK => {
                    let response_headers = response.headers().clone();

                    // Check if the response requires tool call
                    let requires_tool_call = parse_requires_tool_call_header(&response_headers);

                    if requires_tool_call {
                        let tool_calls =
                            extract_tool_calls_from_stream(response, request_id).await?;

                        // Convert tool calls to stored format for memory
                        let stored_tool_calls = conv_id
                            .as_ref()
                            .map(|conv_id| convert_tool_calls_to_stored(&tool_calls, conv_id));

                        call_mcp_server(
                            tool_calls.as_slice(),
                            &mut request,
                            &headers,
                            &chat_server,
                            request_id,
                            cancel_token,
                            conv_id.as_deref(),
                            user_message.as_deref(),
                            &state,
                            stored_tool_calls,
                        )
                        .await
                    } else {
                        // Handle response body reading with cancellation
                        let bytes = select! {
                            bytes = response.bytes() => {
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

                        // Extract assistant message for memory storage in streaming response
                        if let (Some(conv_id), Some(memory)) = (&conv_id, &state.memory) {
                            match std::str::from_utf8(&bytes) {
                                Ok(response_text) => {
                                    match extract_assistant_message_from_stream(response_text) {
                                        Ok(assistant_message) => {
                                            if let Err(e) = memory
                                                .add_assistant_message(
                                                    conv_id,
                                                    &assistant_message,
                                                    vec![],
                                                )
                                                .await
                                            {
                                                dual_error!(
                                                    "Failed to add assistant message to memory: {} - request_id: {}",
                                                    e,
                                                    request_id
                                                );
                                            }
                                        }
                                        Err(e) => {
                                            let warn_msg = format!(
                                                "Failed to extract assistant message from streaming response: {e} - request_id: {request_id}",
                                            );
                                            dual_warn!("{}", warn_msg);
                                        }
                                    }
                                }
                                Err(e) => {
                                    let warn_msg = format!(
                                        "Failed to parse streaming response as UTF-8: {e} - request_id: {request_id}",
                                    );
                                    dual_warn!("{}", warn_msg);
                                }
                            }
                        }

                        // build the response builder
                        let response_builder = Response::builder().status(status);

                        // copy the response headers
                        let response_builder =
                            copy_response_headers(response_builder, &response_headers);

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
            }
        }
        Some(false) | None => {
            let status = response.status();

            // check the status code
            match status {
                StatusCode::OK => {
                    let response_headers = response.headers().clone();

                    // Read the response body
                    let bytes =
                        read_response_bytes(response, request_id, cancel_token.clone()).await?;
                    let chat_completion = parse_chat_completion(&bytes, request_id)?;

                    // Check if the response requires tool call
                    let requires_tool_call =
                        !chat_completion.choices[0].message.tool_calls.is_empty();

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

                        call_mcp_server(
                            chat_completion.choices[0].message.tool_calls.as_slice(),
                            &mut request,
                            &headers,
                            &chat_server,
                            request_id,
                            cancel_token,
                            conv_id.as_deref(),
                            user_message.as_deref(),
                            &state,
                            stored_tool_calls,
                        )
                        .await
                    } else {
                        // 存储助手消息到记忆中
                        if let Some(memory) = &state.memory
                            && let Some(conv_id) = &conv_id
                            && let Some(assistant_msg) = &chat_completion.choices[0].message.content
                            && let Err(e) = memory
                                .add_assistant_message(conv_id, assistant_msg, vec![])
                                .await
                        {
                            dual_error!(
                                "Failed to add assistant message to memory: {} - request_id: {}",
                                e,
                                request_id
                            );
                        }

                        // Handle normal response in non-stream mode
                        build_response(status, response_headers, bytes, request_id)
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
            }
        }
    };

    // ! print full chat history
    if let Some(memory) = &state.memory
        && let Some(conv_id) = &conv_id
    {
        let chat_history = memory.get_full_history(conv_id).await.map_err(|e| {
            let err_msg = format!("Failed to get chat history: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;
        dual_debug!(
            "Full history - request_id: {}\n{}",
            request_id,
            serde_json::to_string_pretty(&chat_history).unwrap()
        );

        let working_messages = memory.get_working_messages(conv_id).await.map_err(|e| {
            let err_msg = format!("Failed to get working messages: {e}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            ServerError::Operation(err_msg)
        })?;
        dual_debug!(
            "Working messages - request_id: {}\n{}",
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
            "Model context - request_id: {}\n{}",
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

/// Extract user messages from the chat request
fn extract_user_message(request: &ChatCompletionRequest) -> Option<String> {
    request.messages.iter().rev().find_map(|msg| {
        match msg {
            endpoints::chat::ChatCompletionRequestMessage::User(user_msg) => {
                match user_msg.content() {
                    ChatCompletionUserMessageContent::Text(text) => Some(text.clone()),
                    ChatCompletionUserMessageContent::Parts(parts) => {
                        // 提取文本部分
                        let text_parts: Vec<String> = parts
                            .iter()
                            .filter_map(|part| {
                                // 简化处理，直接尝试转换为字符串
                                // 这里可能需要根据实际的part类型来处理
                                serde_json::to_string(part).ok()
                            })
                            .collect();
                        if text_parts.is_empty() {
                            None
                        } else {
                            Some(text_parts.join(" "))
                        }
                    }
                }
            }
            _ => None,
        }
    })
}

/// Parse tool call identifier from HTTP response headers
///
/// Check if the "requires-tool-call" field exists in response headers and parse it as boolean.
/// Returns false if the field doesn't exist or parsing fails.
fn parse_requires_tool_call_header(headers: &HeaderMap) -> bool {
    headers
        .get("requires-tool-call")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<bool>().ok())
        .unwrap_or(false)
}

/// Extract tool call information from streaming response
///
/// Parse streaming response data and extract tool call information.
/// Process SSE format data stream, parse ChatCompletionChunk and extract tool_calls.
async fn extract_tool_calls_from_stream(
    response: reqwest::Response,
    request_id: &str,
) -> ServerResult<Vec<ToolCall>> {
    let mut ds_stream = response.bytes_stream();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    while let Some(item) = ds_stream.next().await {
        match item {
            Ok(bytes) => {
                match String::from_utf8(bytes.to_vec()) {
                    Ok(s) => {
                        let x = s
                            .trim_start_matches("data:")
                            .trim()
                            .split("data:")
                            .collect::<Vec<_>>();
                        let s = x[0];

                        dual_debug!("s: {}", s);

                        // convert the bytes to ChatCompletionChunk
                        if let Ok(chunk) = serde_json::from_str::<ChatCompletionChunk>(s) {
                            dual_debug!("chunk: {:?} - request_id: {}", &chunk, request_id);

                            if !chunk.choices.is_empty() {
                                for tool in chunk.choices[0].delta.tool_calls.iter() {
                                    let tool_call = tool.clone().into();

                                    dual_debug!("tool_call: {:?}", &tool_call);

                                    tool_calls.push(tool_call);
                                }

                                break;
                            }
                        }
                    }
                    Err(e) => {
                        let err_msg = format!(
                            "Failed to convert bytes from downstream server into string: {e}"
                        );
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg));
                    }
                }
            }
            Err(e) => {
                let err_msg = format!("Failed to get the full response as bytes: {e}");
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg));
            }
        }
    }

    Ok(tool_calls)
}

/// Convert tool calls from endpoints format to memory format
fn convert_tool_calls_to_stored(
    tool_calls: &[ToolCall],
    _conv_id: &str, // 预留参数，可能用于会话上下文
) -> Vec<StoredToolCall> {
    tool_calls
        .iter()
        .enumerate()
        .map(|(idx, tc)| {
            let arguments = match serde_json::from_str(&tc.function.arguments) {
                Ok(value) => value,
                Err(_) => serde_json::Value::String(tc.function.arguments.clone()),
            };

            StoredToolCall {
                id: tc.id.clone(),
                name: tc.function.name.clone(),
                arguments,
                result: None, // 工具结果稍后添加
                sequence: idx as i32,
            }
        })
        .collect()
}

/// Extract assistant message content from streaming response
///
/// Parse the streaming response text to extract the assistant's message content.
/// Streaming responses are typically in SSE (Server-Sent Events) format.
fn extract_assistant_message_from_stream(response_text: &str) -> ServerResult<String> {
    let mut content_parts = Vec::new();

    // Parse SSE format response
    for line in response_text.lines() {
        if let Some(data_part) = line.strip_prefix("data: ") {
            // Skip [DONE] marker
            if data_part.trim() == "[DONE]" {
                continue;
            }

            // Try to parse as JSON
            if let Ok(chunk) = serde_json::from_str::<serde_json::Value>(data_part)
                && let Some(choices) = chunk.get("choices")
                && let Some(choice) = choices.get(0)
                && let Some(delta) = choice.get("delta")
                && let Some(content) = delta.get("content")
                && let Some(content_str) = content.as_str()
            {
                content_parts.push(content_str.to_string());
            }
        }
    }

    if content_parts.is_empty() {
        return Err(ServerError::Operation(
            "No assistant message content found in streaming response".to_string(),
        ));
    }

    Ok(content_parts.join(""))
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
    tool_calls: &[ToolCall],
    request: &mut ChatCompletionRequest,
    headers: &HeaderMap,
    chat_server: &TargetServerInfo,
    request_id: impl AsRef<str>,
    cancel_token: CancellationToken,
    conv_id: Option<&str>,
    _user_message: Option<&str>,
    state: &Arc<AppState>,
    mut stored_tool_calls: Option<Vec<StoredToolCall>>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();
    let chat_service_url = format!("{}/chat/completions", chat_server.url.trim_end_matches('/'));

    dual_debug!(
        "tool calls:\n{}",
        serde_json::to_string_pretty(tool_calls).unwrap()
    );
    dual_debug!(
        "first tool call:\n{}",
        serde_json::to_string_pretty(&tool_calls[0]).unwrap()
    );

    let tool_call = &tool_calls[0];
    let tool_call_id = tool_call.id.as_str();
    let tool_name = tool_call.function.name.as_str();
    let tool_args = &tool_call.function.arguments;

    dual_debug!(
        "tool name: {}, tool args: {} - request_id: {}",
        tool_name,
        tool_args,
        request_id
    );

    // convert the func_args to a json object
    let arguments =
        serde_json::from_str::<serde_json::Map<String, serde_json::Value>>(tool_args).ok();

    // find mcp client by tool name
    if let Some(mcp_tools) = MCP_TOOLS.get() {
        let tools = mcp_tools.read().await;
        dual_debug!("mcp_tools: {:?}", mcp_tools);

        // look up the tool name in MCP_TOOLS
        if let Some(mcp_client_name) = tools.get(tool_name) {
            if let Some(services) = MCP_SERVICES.get() {
                let service_map = services.read().await;
                // get the mcp client
                let service = match service_map.get(mcp_client_name) {
                    Some(mcp_client) => mcp_client,
                    None => {
                        let err_msg = format!("Tool not found: {tool_name}");
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg.to_string()));
                    }
                };

                // get the server name from the peer info
                let raw_server_name = match service.read().await.raw.peer_info() {
                    Some(peer_info) => {
                        let server_name = peer_info.server_info.name.clone();
                        dual_debug!(
                            "server name from peer info: {} - request_id: {}",
                            server_name,
                            request_id
                        );
                        server_name
                    }
                    None => {
                        dual_warn!(
                            "Failed to get peer info from the MCP client: {mcp_client_name}"
                        );

                        String::new()
                    }
                };

                dual_info!(
                    "Call `{}::{}` mcp tool - request_id: {}",
                    raw_server_name,
                    tool_name,
                    request_id
                );

                // call a tool
                let request_param = CallToolRequestParam {
                    name: tool_name.to_string().into(),
                    arguments,
                };
                let res = service
                    .read()
                    .await
                    .raw
                    .call_tool(request_param)
                    .await
                    .map_err(|e| {
                        dual_error!("Failed to call the tool: {}", e);
                        ServerError::Operation(e.to_string())
                    })?;
                dual_debug!("{}", serde_json::to_string_pretty(&res).unwrap());

                match res.is_error {
                    Some(false) => {
                        match &res.content {
                            None => {
                                let err_msg = "The mcp tool result is empty";
                                dual_error!("{} - request_id: {}", err_msg, request_id);
                                Err(ServerError::McpEmptyContent)
                            }
                            Some(content) => {
                                let content = &content[0];
                                match &content.raw {
                                    RawContent::Text(text) => {
                                        dual_info!("The mcp tool call result: {:#?}", text.text);

                                        // 存储工具调用及结果到记忆中
                                        if let (Some(conv_id), Some(stored_tcs), Some(memory)) =
                                            (conv_id, stored_tool_calls.as_mut(), &state.memory)
                                        {
                                            // Add tool results to stored tool calls
                                            add_tool_results_to_stored(
                                                stored_tcs,
                                                std::slice::from_ref(&text.text),
                                            );

                                            if let Err(e) = memory
                                                .add_assistant_message(
                                                    conv_id,
                                                    "",
                                                    stored_tcs.clone(),
                                                )
                                                .await
                                            {
                                                dual_error!(
                                                    "Failed to store tool calls to memory: {} - request_id: {}",
                                                    e,
                                                    request_id
                                                );
                                            }
                                        }

                                        match SEARCH_MCP_SERVER_NAMES
                                            .contains(&raw_server_name.as_str())
                                        {
                                            true => {
                                                // get the fallback message from the mcp client
                                                let fallback = if service
                                                    .read()
                                                    .await
                                                    .has_fallback_message()
                                                {
                                                    service
                                                        .read()
                                                        .await
                                                        .fallback_message
                                                        .clone()
                                                        .unwrap()
                                                } else {
                                                    DEFAULT_SEARCH_FALLBACK_MESSAGE.to_string()
                                                };

                                                dual_debug!(
                                                    "fallback message: {} - request_id: {}",
                                                    fallback,
                                                    request_id
                                                );

                                                // format the content
                                                let content = format!(
                                                    "Please answer the question based on the information between **---BEGIN CONTEXT---** and **---END CONTEXT---**. Do not use any external knowledge. If the information between **---BEGIN CONTEXT---** and **---END CONTEXT---** is empty, please respond with `{fallback}`. Note that DO NOT use any tools if provided.\n\n---BEGIN CONTEXT---\n\n{context}\n\n---END CONTEXT---",
                                                    fallback = fallback,
                                                    context = &text.text,
                                                );

                                                // append assistant message with tool call to request messages
                                                let assistant_completion_message =
                                                    ChatCompletionRequestMessage::Assistant(
                                                        ChatCompletionAssistantMessage::new(
                                                            None,
                                                            None,
                                                            Some(tool_calls.to_vec()),
                                                        ),
                                                    );
                                                request.messages.push(assistant_completion_message);

                                                // append tool message with tool result to request messages
                                                let tool_completion_message =
                                                    ChatCompletionRequestMessage::Tool(
                                                        ChatCompletionToolMessage::new(
                                                            &content,
                                                            tool_call_id,
                                                        ),
                                                    );
                                                request.messages.push(tool_completion_message);

                                                // disable tool choice
                                                if request.tool_choice.is_some() {
                                                    request.tool_choice = Some(ToolChoice::None);
                                                }

                                                // Create a request client that can be cancelled
                                                let ds_request = if let Some(api_key) =
                                                    &chat_server.api_key
                                                    && !api_key.is_empty()
                                                {
                                                    reqwest::Client::new()
                                                        .post(&chat_service_url)
                                                        .header(CONTENT_TYPE, "application/json")
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

                                                dual_info!(
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

                                                let mut response_builder =
                                                    Response::builder().status(status);

                                                // Copy all headers from downstream response
                                                match request.stream {
                                                    Some(true) => {
                                                        for (name, value) in headers.iter() {
                                                            match name.as_str() {
                                                                "access-control-allow-origin" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "access-control-allow-headers" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "access-control-allow-methods" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "content-type" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "cache-control" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "connection" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "user" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                "date" => {
                                                                    response_builder =
                                                                        response_builder
                                                                            .header(name, value);
                                                                }
                                                                _ => {
                                                                    dual_debug!(
                                                                        "ignore header: {} - {}",
                                                                        name,
                                                                        value.to_str().unwrap()
                                                                    );
                                                                }
                                                            }
                                                        }
                                                    }
                                                    Some(false) | None => {
                                                        for (name, value) in headers.iter() {
                                                            dual_debug!(
                                                                "{}: {}",
                                                                name,
                                                                value.to_str().unwrap()
                                                            );
                                                            response_builder = response_builder
                                                                .header(name, value);
                                                        }
                                                    }
                                                }

                                                match response_builder
                                                    .body(Body::from(bytes.clone()))
                                                {
                                                    Ok(response) => {
                                                        if let (Some(conv_id), Some(memory)) =
                                                            (conv_id, &state.memory)
                                                        {
                                                            // 存储最终的助手消息到记忆中
                                                            match request.stream {
                                                                Some(true) => {
                                                                    match std::str::from_utf8(&bytes) {
                                                                        Ok(response_text) => {
                                                                            match extract_assistant_message_from_stream(response_text) {
                                                                                Ok(assistant_message) => {
                                                                                    if let Err(e) = memory.add_assistant_message(conv_id, &assistant_message, vec![]).await {
                                                                                        dual_error!("Failed to add assistant message to memory: {e} - request_id: {}", request_id);
                                                                                    }
                                                                                }
                                                                                Err(e) => {
                                                                                    let warn_msg = format!("Failed to extract assistant message from stream: {e}");
                                                                                    dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                                                }
                                                                            }
                                                                        },
                                                                        Err(e) => {
                                                                            let warn_msg = format!("Failed to parse SSE stream: {e}");
                                                                            dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                                        }
                                                                    }
                                                                },
                                                                Some(false) | None => {
                                                                    match parse_chat_completion(&bytes, request_id) {
                                                                        Ok(chat_completion) => {
                                                                            if let Some(assistant_message) = chat_completion.choices.first().and_then(|choice| choice.message.content.clone()) && let Err(e) = memory.add_assistant_message(conv_id, &assistant_message, vec![]).await {
                                                                                dual_error!("Failed to add assistant message to memory: {e} - request_id: {}", request_id);
                                                                            }
                                                                        },
                                                                        Err(e) => {
                                                                            let warn_msg = format!("Failed to parse chat completion: {e}");
                                                                            dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                                        }
                                                                    }
                                                                }
                                                            };
                                                        }

                                                        dual_info!(
                                                            "Chat request completed successfully - request_id: {}",
                                                            request_id
                                                        );

                                                        Ok(response)
                                                    }
                                                    Err(e) => {
                                                        let err_msg = format!(
                                                            "Failed to create the response: {e}"
                                                        );
                                                        dual_error!(
                                                            "{} - request_id: {}",
                                                            err_msg,
                                                            request_id
                                                        );
                                                        Err(ServerError::Operation(err_msg))
                                                    }
                                                }
                                            }
                                            false => {
                                                // create an assistant message
                                                let tool_completion_message =
                                                    ChatCompletionRequestMessage::Tool(
                                                        ChatCompletionToolMessage::new(
                                                            &text.text,
                                                            tool_call_id,
                                                        ),
                                                    );

                                                // append assistant message with tool call to request messages
                                                let assistant_completion_message =
                                                    ChatCompletionRequestMessage::Assistant(
                                                        ChatCompletionAssistantMessage::new(
                                                            None,
                                                            None,
                                                            Some(tool_calls.to_vec()),
                                                        ),
                                                    );
                                                request.messages.push(assistant_completion_message);
                                                // append tool message with tool result to request messages
                                                request.messages.push(tool_completion_message);

                                                // disable tool choice
                                                if request.tool_choice.is_some() {
                                                    request.tool_choice = Some(ToolChoice::None);
                                                }

                                                // Create a request client that can be cancelled
                                                let ds_request = if let Some(api_key) =
                                                    &chat_server.api_key
                                                    && !api_key.is_empty()
                                                {
                                                    reqwest::Client::new()
                                                        .post(&chat_service_url)
                                                        .header(CONTENT_TYPE, "application/json")
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
                                                let mut response_builder =
                                                    Response::builder().status(status);

                                                // copy the response headers
                                                response_builder = copy_response_headers(
                                                    response_builder,
                                                    ds_response.headers(),
                                                );

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

                                                match response_builder
                                                    .body(Body::from(bytes.clone()))
                                                {
                                                    Ok(response) => {
                                                        if let (Some(conv_id), Some(memory)) =
                                                            (conv_id, &state.memory)
                                                        {
                                                            // 存储最终的助手消息到记忆中
                                                            match request.stream {
                                                                Some(true) => {
                                                                    match std::str::from_utf8(&bytes) {
                                                                        Ok(response_text) => {
                                                                            match extract_assistant_message_from_stream(response_text) {
                                                                                Ok(assistant_message) => {
                                                                                    if let Err(e) = memory.add_assistant_message(conv_id, &assistant_message, vec![]).await {
                                                                                        dual_warn!("Failed to add assistant message to memory: {e} - request_id: {}", request_id);
                                                                                    }
                                                                                }
                                                                                Err(e) => {
                                                                                    let warn_msg = format!("Failed to extract assistant message from stream: {e}");
                                                                                    dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                                                }
                                                                            }
                                                                        },
                                                                        Err(e) => {
                                                                            let warn_msg = format!("Failed to parse SSE stream: {e}");
                                                                            dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                                        }
                                                                    }
                                                                },
                                                                Some(false) | None => {
                                                                    match parse_chat_completion(&bytes, request_id) {
                                                                        Ok(chat_completion) => {
                                                                            if let Some(assistant_message) = chat_completion.choices.first().and_then(|choice| choice.message.content.clone()) && let Err(e) = memory.add_assistant_message(conv_id, &assistant_message, vec![]).await {
                                                                                dual_warn!("Failed to add assistant message to memory: {e} - request_id: {}", request_id);
                                                                            }

                                                                        },
                                                                        Err(e) => {
                                                                            let warn_msg = format!("Failed to parse chat completion: {e}");
                                                                            dual_warn!("{} - request_id: {}", warn_msg, request_id);
                                                                        }
                                                                    }
                                                                }
                                                            };
                                                        }

                                                        dual_info!(
                                                            "Chat request completed successfully - request_id: {}",
                                                            request_id
                                                        );

                                                        Ok(response)
                                                    }
                                                    Err(e) => {
                                                        let err_msg = format!(
                                                            "Failed to create the response: {e}"
                                                        );
                                                        dual_error!(
                                                            "{} - request_id: {}",
                                                            err_msg,
                                                            request_id
                                                        );
                                                        Err(ServerError::Operation(err_msg))
                                                    }
                                                }
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
                        }
                    }
                    _ => {
                        let err_msg = format!("Failed to call the tool: {tool_name}");
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        Err(ServerError::Operation(err_msg))
                    }
                }
            } else {
                let err_msg = "Empty MCP CLIENTS";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                Err(ServerError::Operation(err_msg.to_string()))
            }
        } else {
            let err_msg = format!("Failed to find the MCP client with tool name: {tool_name}");
            dual_error!("{} - request_id: {}", err_msg, request_id);
            Err(ServerError::McpNotFoundClient)
        }
    } else {
        let err_msg = "Empty MCP TOOLS";
        dual_error!("{} - request_id: {}", err_msg, request_id);
        Err(ServerError::Operation(err_msg.to_string()))
    }
}

/// Add tool results to stored tool calls
fn add_tool_results_to_stored(
    stored_tool_calls: &mut [StoredToolCall],
    tool_results: &[String], // 简化的工具结果
) {
    for (stored_tc, result) in stored_tool_calls.iter_mut().zip(tool_results.iter()) {
        stored_tc.result = Some(StoredToolResult {
            content: serde_json::Value::String(result.clone()),
            success: true,
            error: None,
            execution_time_ms: None,
            timestamp: chrono::Utc::now(),
        });
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
                        // 如果存在工具调用，则将其包含在请求消息中
                        // ChatCompletionRequestMessage::Assistant(
                        //     ChatCompletionAssistantMessage::new(msg.content, None, Some(tool_call.clone())),
                        // )

                        let tool_calls = tool_calls
                            .into_iter()
                            .map(|tool_call| tool_call.into())
                            .collect();

                        ChatCompletionRequestMessage::new_assistant_message(
                            Some(msg.content),
                            None,
                            Some(tool_calls),
                        )
                    }
                    None => {
                        // 否则只包含消息内容
                        // ChatCompletionRequestMessage::Assistant(
                        //     ChatCompletionAssistantMessage::new(msg.content, None, None),
                        // )

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
