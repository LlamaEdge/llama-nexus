use endpoints::chat::{ChatCompletionRequest, ChatCompletionUserMessageContent, ToolCall};

use crate::memory::{StoredToolCall, StoredToolResult};

/// Extract user messages from the chat request
pub(super) fn extract_user_message(request: &ChatCompletionRequest) -> Option<String> {
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

/// Extract system message from the chat request
pub(super) fn extract_system_message(request: &ChatCompletionRequest) -> Option<String> {
    request.messages.iter().find_map(|msg| match msg {
        endpoints::chat::ChatCompletionRequestMessage::System(system_msg) => {
            Some(system_msg.content().to_string())
        }
        _ => None,
    })
}

/// Add tool results to stored tool calls
pub(super) fn add_tool_results_to_stored(
    stored_tool_calls: &mut [StoredToolCall],
    tool_results: &[String],
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

/// Convert tool calls from endpoints format to memory format
pub(super) fn convert_tool_calls_to_stored(
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
