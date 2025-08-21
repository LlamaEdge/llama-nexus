use std::{sync::Arc, time::SystemTime};

use axum::{
    Json,
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, Response, StatusCode},
};
use endpoints::{
    chat::{
        ChatCompletionAssistantMessage, ChatCompletionChunk, ChatCompletionChunkChoice,
        ChatCompletionChunkChoiceDelta, ChatCompletionObject, ChatCompletionRequest,
        ChatCompletionRequestMessage, ChatCompletionRole, ChatCompletionToolMessage,
    },
    common::FinishReason,
};
use futures_util::{
    StreamExt,
    stream::{self},
};
use regex::Regex;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use rmcp::model::{CallToolRequestParam, RawContent};
use tokio::select;
use tokio_util::sync::CancellationToken;

use crate::{
    AppState,
    chat::gen_chat_id,
    dual_debug, dual_error, dual_info, dual_warn,
    error::{ServerError, ServerResult},
    mcp::{DEFAULT_SEARCH_FALLBACK_MESSAGE, MCP_SERVICES, MCP_TOOLS, SEARCH_MCP_SERVER_NAMES},
    server::{RoutingPolicy, ServerKind, TargetServerInfo},
};

#[allow(dead_code)]
pub(crate) async fn chat(
    State(state): State<Arc<AppState>>,
    Extension(cancel_token): Extension<CancellationToken>,
    headers: HeaderMap,
    Json(mut request): Json<ChatCompletionRequest>,
    request_id: impl AsRef<str>,
) -> ServerResult<axum::response::Response> {
    let request_id = request_id.as_ref();

    // Get target server
    let chat_server = get_chat_server(&state, request_id).await?;

    run_in_react_mode(
        &chat_server,
        &mut request,
        &headers,
        request_id,
        cancel_token.clone(),
    )
    .await
}

#[allow(dead_code)]
async fn run_in_react_mode(
    chat_server: &TargetServerInfo,
    request: &mut ChatCompletionRequest,
    headers: &HeaderMap,
    request_id: &str,
    cancel_token: CancellationToken,
) -> ServerResult<axum::response::Response> {
    let action_pattern = Regex::new(r"<action>(.*?)</action>").unwrap();
    let thought_pattern = Regex::new(r"<thought>(.*?)</thought>").unwrap();
    let final_answer_pattern = Regex::new(r"<final_answer>(.*?)</final_answer>").unwrap();

    loop {
        // * build request
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
            serde_json::to_string_pretty(request).unwrap()
        );

        // * send request to downstream server

        // Use select! to support cancellation
        let ds_response = select! {
            response = client.json(request).send() => {
                response.map_err(|e| ServerError::Operation(format!("Failed to forward request: {e}")))
            }
            _ = cancel_token.cancelled() => {
                let warn_msg = "Request was cancelled by client";
                dual_warn!("{}", warn_msg);
                Err(ServerError::Operation(warn_msg.to_string()))
            }
        }?;

        // get the response body
        let mut chat_completion =
            ds_response
                .json::<ChatCompletionObject>()
                .await
                .map_err(|e| {
                    let err_msg = format!("Failed to get the response body: {e}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    ServerError::Operation(err_msg)
                })?;

        dual_debug!(
            "chat completion:\n{}",
            serde_json::to_string_pretty(&chat_completion).unwrap()
        );

        if !chat_completion.choices[0].message.tool_calls.is_empty() {
            if let Some(content) = chat_completion.choices[0].message.content.as_ref() {
                // 检测 <thought> 标签
                if content.contains("<thought>") {
                    // get the text between <thought> and </thought>
                    let thought = thought_pattern
                        .captures(content)
                        .unwrap()
                        .get(1)
                        .unwrap()
                        .as_str();
                    dual_info!("💭 Thought: {}", thought);
                }

                // 检测 <action> 标签
                match action_pattern.captures(content) {
                    Some(captures) => {
                        let action = captures.get(1).unwrap().as_str();
                        dual_info!("🔧 Action: {}", action);
                    }
                    None => {
                        let err_msg = "No <action> tags found in the response";
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg.to_string()));
                    }
                }
            }

            // * call MCP server to execute the action

            let tool_calls = chat_completion.choices[0].message.tool_calls.as_slice();

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
                                        return Err(ServerError::McpEmptyContent);
                                    }
                                    Some(content) => {
                                        let content = content[0].clone();
                                        match &content.raw {
                                            RawContent::Text(text) => {
                                                dual_debug!(
                                                    "The mcp tool call result: {:#?}",
                                                    text.text
                                                );

                                                match SEARCH_MCP_SERVER_NAMES
                                                    .contains(&raw_server_name.as_str())
                                                {
                                                    true => {
                                                        dual_info!(
                                                            "🔍 Observation:\n{}",
                                                            &text.text
                                                        );

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
                                                            DEFAULT_SEARCH_FALLBACK_MESSAGE
                                                                .to_string()
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
                                                        request
                                                            .messages
                                                            .push(assistant_completion_message);

                                                        // append tool message with tool result to request messages
                                                        let content = format!(
                                                            "<observation>{}</observation>",
                                                            &content
                                                        );
                                                        let tool_completion_message =
                                                            ChatCompletionRequestMessage::Tool(
                                                                ChatCompletionToolMessage::new(
                                                                    &content,
                                                                    tool_call_id,
                                                                ),
                                                            );
                                                        request
                                                            .messages
                                                            .push(tool_completion_message);
                                                    }
                                                    false => {
                                                        dual_info!(
                                                            "🔍 Observation: {}",
                                                            &text.text
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
                                                        request
                                                            .messages
                                                            .push(assistant_completion_message);

                                                        // append tool message with tool result to request messages

                                                        let content = format!(
                                                            "<observation>{}</observation>",
                                                            &text.text
                                                        );
                                                        let tool_completion_message =
                                                            ChatCompletionRequestMessage::Tool(
                                                                ChatCompletionToolMessage::new(
                                                                    &content,
                                                                    tool_call_id,
                                                                ),
                                                            );
                                                        request
                                                            .messages
                                                            .push(tool_completion_message);
                                                    }
                                                }
                                            }
                                            _ => {
                                                let err_msg = "Only text content is supported for tool call results";
                                                dual_error!(
                                                    "{} - request_id: {}",
                                                    err_msg,
                                                    request_id
                                                );
                                                return Err(ServerError::Operation(
                                                    err_msg.to_string(),
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                            _ => {
                                let err_msg = format!("Failed to call the tool: {tool_name}");
                                dual_error!("{} - request_id: {}", err_msg, request_id);
                                return Err(ServerError::Operation(err_msg));
                            }
                        }
                    } else {
                        let err_msg = "Empty MCP CLIENTS";
                        dual_error!("{} - request_id: {}", err_msg, request_id);
                        return Err(ServerError::Operation(err_msg.to_string()));
                    }
                } else {
                    let err_msg =
                        format!("Failed to find the MCP client with tool name: {tool_name}");
                    dual_error!("{} - request_id: {}", err_msg, request_id);
                    return Err(ServerError::McpNotFoundClient);
                }
            } else {
                let err_msg = "Empty MCP TOOLS";
                dual_error!("{} - request_id: {}", err_msg, request_id);
                return Err(ServerError::Operation(err_msg.to_string()));
            }
        } else {
            match chat_completion.choices[0].message.content.as_ref() {
                Some(content) => {
                    // 检测 <thought> 标签
                    if content.contains("<thought>") {
                        // get the text between <thought> and </thought>
                        let thought = thought_pattern
                            .captures(content)
                            .unwrap()
                            .get(1)
                            .unwrap()
                            .as_str();
                        dual_info!("💭 Thought: {}", thought);
                    }

                    // 检测 <final_answer> 标签
                    if content.contains("<final_answer>") {
                        // get the text between <final_answer> and </final_answer>
                        let final_answer = final_answer_pattern
                            .captures(content)
                            .unwrap()
                            .get(1)
                            .unwrap()
                            .as_str()
                            .to_string(); // 转换为String避免借用问题
                        dual_info!("Final answer: {}", final_answer);

                        match request.stream {
                            Some(true) => {
                                let chunks: Vec<String> = final_answer
                                    .split_whitespace()
                                    .map(|s| s.to_string())
                                    .collect();
                                let id = match &request.user {
                                    Some(id) => id.clone(),
                                    None => gen_chat_id(),
                                };
                                let model = chat_completion.model.clone();
                                let usage = chat_completion.usage;
                                let chunks_len = chunks.len();

                                // 创建SSE流
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

                                // 构建流式响应
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
                                        let err_msg =
                                            format!("Failed to create streaming response: {e}");
                                        dual_error!("{} - request_id: {}", err_msg, request_id);
                                        return Err(ServerError::Operation(err_msg));
                                    }
                                }
                            }
                            _ => {
                                chat_completion.choices[0].message.content =
                                    Some(final_answer.to_string());
                                let response_body =
                                    serde_json::to_string(&chat_completion).unwrap();

                                // build the response builder
                                let response_builder = Response::builder()
                                    .header(CONTENT_TYPE, "application/json")
                                    .status(StatusCode::OK);

                                match response_builder.body(Body::from(response_body)) {
                                    Ok(response) => {
                                        return Ok(response);
                                    }
                                    Err(e) => {
                                        let err_msg = format!("Failed to create the response: {e}");
                                        dual_error!("{} - request_id: {}", err_msg, request_id);
                                        return Err(ServerError::Operation(err_msg));
                                    }
                                }
                            }
                        }
                    }

                    // 检测 <action> 标签
                    match action_pattern.captures(content) {
                        Some(captures) => {
                            let action = captures.get(1).unwrap().as_str();
                            dual_info!("🔧 Action: {}", action);
                        }
                        None => {
                            let err_msg = "No <action> tags found in the response";
                            dual_error!("{} - request_id: {}", err_msg, request_id);
                            return Err(ServerError::Operation(err_msg.to_string()));
                        }
                    }
                }
                None => {
                    todo!()
                }
            }
        }
    }
}

#[allow(dead_code)]
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
