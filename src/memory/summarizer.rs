use axum::http::StatusCode;
use endpoints::chat::{
    ChatCompletionObject, ChatCompletionRequestBuilder, ChatCompletionRequestMessage,
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};

use crate::{config::SummarizationStrategy, dual_debug, dual_info, memory::types::*};

/// 消息摘要生成器
///
/// 用于将对话中的多条消息压缩成简洁的摘要，以节省上下文空间。
/// 支持两种摘要策略：
/// 1. 增量摘要：基于现有摘要和新消息生成更新的摘要（效率更高）
/// 2. 完整历史摘要：基于所有历史消息重新生成摘要（上下文更完整）
#[allow(dead_code)]
pub struct MessageSummarizer {
    // 这里可以配置使用哪个模型进行摘要
    model_name: Option<String>,
    summary_service_base_url: String,
    summary_service_api_key: String,
    /// 摘要策略配置
    summarization_strategy: SummarizationStrategy,
}

impl MessageSummarizer {
    /// 创建一个新的消息摘要生成器实例
    ///
    /// # 参数
    /// * `model_name` - 可选的模型名称，用于指定使用哪个 LLM 进行摘要生成
    /// * `summary_service_base_url` - 摘要服务的基础 URL
    /// * `summary_service_api_key` - 摘要服务的 API 密钥
    /// * `summarization_strategy` - 摘要策略（增量摘要或完整历史摘要）
    ///
    /// # 返回值
    /// * `Self` - MessageSummarizer 实例
    ///
    /// # 说明
    /// 如果不指定模型名称，将使用默认模型进行摘要生成。
    pub fn new(
        model_name: Option<String>,
        summary_service_base_url: impl AsRef<str>,
        summary_service_api_key: impl AsRef<str>,
        summarization_strategy: SummarizationStrategy,
    ) -> Self {
        Self {
            model_name,
            summary_service_base_url: summary_service_base_url.as_ref().to_string(),
            summary_service_api_key: summary_service_api_key.as_ref().to_string(),
            summarization_strategy,
        }
    }

    /// 为存储的消息生成摘要
    ///
    /// # 参数
    /// * `messages` - 需要摘要的消息列表
    /// * `existing_summary` - 可选的现有摘要，用于增量摘要生成
    /// * `full_history_messages` - 可选的完整历史消息，用于完整历史摘要策略
    ///
    /// # 返回值
    /// * `MemoryResult<String>` - 成功时返回生成的摘要文本，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法支持两种摘要策略：
    /// 1. 增量摘要：基于现有摘要和新消息生成更新的摘要
    /// 2. 完整历史摘要：基于所有历史消息重新生成摘要
    ///
    /// 摘要包含主要话题、关键决策、使用的工具以及未解决的问题。
    ///
    /// # 错误
    /// * `MemoryError::SummarizationFailed` - 当 LLM API 调用失败时
    pub async fn summarize_stored_messages(
        &self,
        messages: &[StoredMessage],
        existing_summary: Option<&str>,
        full_history_messages: Option<&[StoredMessage]>,
    ) -> MemoryResult<String> {
        dual_info!(
            "Summarizing stored messages using strategy: {}",
            self.summarization_strategy
        );

        if messages.is_empty() {
            return Ok(existing_summary.unwrap_or("").to_string());
        }

        let prompt = match self.summarization_strategy {
            SummarizationStrategy::Incremental => {
                self.build_incremental_summary_prompt(messages, existing_summary)
            }
            SummarizationStrategy::FullHistory => {
                self.build_full_history_summary_prompt(full_history_messages.unwrap_or(messages))
            }
        };

        dual_debug!("Prompt for summary generation:\n{}", prompt);

        // 调用LLM API来生成摘要
        self.generate_summary_via_llm(&prompt).await
    }

    /// 构建增量摘要提示词
    ///
    /// # 参数
    /// * `messages` - 新增的消息列表
    /// * `existing_summary` - 现有的摘要
    ///
    /// # 返回值
    /// * `String` - 构建的提示词
    fn build_incremental_summary_prompt(
        &self,
        messages: &[StoredMessage],
        existing_summary: Option<&str>,
    ) -> String {
        let mut prompt = String::new();

        if let Some(prev_summary) = existing_summary {
            prompt.push_str(&format!(
                "Previous conversation summary:\\n{prev_summary}\\n\\nNew messages to incorporate:\\n\\n",
            ));
        } else {
            prompt.push_str("Please summarize the following conversation:\\n\\n");
        }

        self.add_messages_to_prompt(&mut prompt, messages);
        self.add_summary_requirements(&mut prompt);

        prompt
    }

    /// 构建完整历史摘要提示词
    ///
    /// # 参数
    /// * `all_history_messages` - 所有需要摘要的历史消息
    ///
    /// # 返回值
    /// * `String` - 构建的提示词
    fn build_full_history_summary_prompt(&self, all_history_messages: &[StoredMessage]) -> String {
        let mut prompt = String::new();

        prompt.push_str(
            "Please create a comprehensive summary of the following conversation history:\\n\\n",
        );

        self.add_messages_to_prompt(&mut prompt, all_history_messages);
        self.add_summary_requirements(&mut prompt);

        prompt
    }

    /// 将消息添加到提示词中
    ///
    /// # 参数
    /// * `prompt` - 提示词字符串（可变引用）
    /// * `messages` - 要添加的消息列表
    fn add_messages_to_prompt(&self, prompt: &mut String, messages: &[StoredMessage]) {
        for msg in messages {
            prompt.push_str(&format!("**{}:** {}\\n", msg.role, msg.content));

            // 包含工具调用信息
            for tool_call in &msg.tool_calls {
                prompt.push_str(&format!("  Used tool: {}", tool_call.name));
                if let Some(result) = &tool_call.result {
                    if result.success {
                        prompt.push_str(" (successful)\\n");
                    } else {
                        prompt.push_str(&format!(
                            " (failed: {})\\n",
                            result.error.as_deref().unwrap_or("unknown error")
                        ));
                    }
                } else {
                    prompt.push_str("\\n");
                }
            }
            prompt.push('\n');
        }
    }

    /// 添加摘要要求到提示词中
    ///
    /// # 参数
    /// * `prompt` - 提示词字符串（可变引用）
    fn add_summary_requirements(&self, prompt: &mut String) {
        prompt.push_str(
            "\\nProvide a concise summary that captures:\\n\\
             1. Main topics discussed\\n\\
             2. Key decisions or conclusions\\n\\
             3. Tools used and their purposes\\n\\
             4. Any unresolved issues\\n\\n\\
             Summary:",
        );
    }

    async fn generate_summary_via_llm(&self, prompt: impl AsRef<str>) -> MemoryResult<String> {
        let user_message = ChatCompletionRequestMessage::new_user_message(
            endpoints::chat::ChatCompletionUserMessageContent::Text(prompt.as_ref().to_string()),
            None,
        );
        let chat_completion = ChatCompletionRequestBuilder::new(&[user_message])
            .with_max_completion_tokens(8192)
            .build();

        // 构造API请求
        let url = format!(
            "{}/chat/completions",
            &self.summary_service_base_url.trim_end_matches('/')
        );

        let mut request = reqwest::Client::new()
            .post(&url)
            .header(CONTENT_TYPE, "application/json")
            .json(&chat_completion);

        if !self.summary_service_api_key.is_empty() {
            request = request.header(
                AUTHORIZATION,
                format!("Bearer {}", &self.summary_service_api_key),
            );
        }

        let response = request.send().await.map_err(|e| {
            MemoryError::SummarizationFailed(format!("Failed to forward request: {e}"))
        })?;

        // 解析响应
        match response.status() {
            StatusCode::OK => {
                let bytes = response.bytes().await.map_err(|e| {
                    let err_msg = format!("Failed to get the full response as bytes: {e}");
                    MemoryError::SummarizationFailed(err_msg)
                })?;

                let chat_completion: ChatCompletionObject = serde_json::from_slice(&bytes)
                    .map_err(|e| {
                        let err_msg = format!("Failed to parse the response: {e}");
                        MemoryError::SummarizationFailed(err_msg)
                    })?;

                let summary = chat_completion.choices[0]
                    .message
                    .content
                    .as_deref()
                    .unwrap();

                Ok(summary.to_string())
            }
            _ => {
                // Convert reqwest::Response to axum::Response
                let status = response.status();

                let err_msg = format!("Failed to generate summary from LLM: {status}");

                Err(MemoryError::SummarizationFailed(err_msg))
            }
        }
    }
}
