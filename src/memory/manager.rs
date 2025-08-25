use std::collections::HashMap;

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    config::MemoryConfig,
    dual_debug, dual_info,
    memory::{store::MessageStore, summarizer::MessageSummarizer, types::*},
};

/// 完整的聊天记忆管理器
///
/// 提供对话的完整生命周期管理，包括消息存储、上下文管理、自动摘要等功能。
/// 该结构体是整个记忆系统的核心，负责协调存储层、摘要器和配置管理。
///
/// # 特性
/// * 分层存储：完整的数据库存储 + 工作上下文缓存
/// * 智能摘要：当上下文过长时自动生成摘要以节省空间
/// * 配置驱动：支持通过配置文件自定义行为参数
/// * 并发安全：使用异步锁保证多线程安全
pub struct CompleteChatMemory {
    /// 底层消息存储，负责持久化数据到 SQLite 数据库
    ///
    /// 提供完整的 CRUD 操作，包括消息存储、对话管理、统计查询等。
    /// 所有的对话和消息数据都通过此组件进行持久化存储。
    store: MessageStore,

    /// 上下文缓存，存储每个对话的工作上下文
    ///
    /// Key: 对话 ID (String)
    /// Value: 对话的上下文记忆 (ContextMemory)
    ///
    /// 使用异步互斥锁保证并发安全，缓存当前活跃对话的工作消息集合，
    /// 避免每次都从数据库加载完整历史。当上下文过长时会触发自动摘要和截断。
    context_cache: Mutex<HashMap<String, ContextMemory>>,

    /// 消息摘要生成器，用于压缩长对话历史
    ///
    /// 当对话上下文超过配置的长度限制时，使用此组件将旧消息
    /// 压缩成摘要文本，以节省上下文空间并保持重要信息。
    /// 支持增量摘要，可以基于现有摘要和新消息生成更新的摘要。
    summarizer: MessageSummarizer,

    /// 记忆系统配置参数
    ///
    /// 包含各种可配置的行为参数，如：
    /// - 数据库文件路径
    /// - 上下文窗口大小限制
    /// - 自动摘要触发条件
    /// - 最大存储消息数量等
    ///
    /// 通过配置文件加载，支持运行时自定义系统行为。
    config: MemoryConfig,
}

impl CompleteChatMemory {
    /// 创建一个新的完整聊天记忆管理器实例
    ///
    /// # 参数
    /// * `config` - 记忆系统配置，包含数据库路径、上下文窗口大小等设置
    ///
    /// # 返回值
    /// * `MemoryResult<Self>` - 成功时返回管理器实例，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法会：
    /// 1. 初始化底层的消息存储（MessageStore）
    /// 2. 创建消息摘要器（MessageSummarizer）
    /// 3. 初始化上下文缓存
    /// 4. 应用配置参数
    ///
    /// # 错误
    /// * `MemoryError::DatabaseError` - 当数据库连接或初始化失败时
    pub async fn new(config: MemoryConfig) -> MemoryResult<Self> {
        // 初始化消息存储
        let store = MessageStore::new(&config.database_path).await?;

        // 创建消息摘要器
        let summarizer = MessageSummarizer::new(
            None,
            &config.summary_service_base_url,
            &config.summary_service_api_key,
            config.summarization_strategy,
        );

        Ok(Self {
            store,
            context_cache: Mutex::new(HashMap::new()),
            summarizer,
            config,
        })
    }

    // 配置映射辅助方法
    fn max_context_tokens(&self) -> usize {
        self.config.context_window as usize
    }

    fn max_working_messages(&self) -> usize {
        self.config.max_stored_messages as usize
    }

    fn enable_summarization(&self) -> bool {
        self.config.auto_summarize
    }

    fn summary_trigger_ratio(&self) -> f32 {
        // 当消息数量超过threshold时触发摘要，设置为0.8的比例
        0.8
    }

    fn keep_recent_messages(&self) -> usize {
        // 保留最近的消息数量，设置为threshold的一半
        (self.config.summarize_threshold / 2) as usize
    }

    /// 创建一个新的对话
    ///
    /// # 参数
    /// * `model_name` - 对话使用的模型名称
    /// * `user_id` - 可选的用户ID
    /// * `title` - 可选的对话标题
    ///
    /// # 返回值
    /// * `MemoryResult<String>` - 成功时返回新创建的对话 ID，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法会：
    /// 1. 生成唯一的对话 ID
    /// 2. 在数据库中创建对话记录
    /// 3. 初始化对话的上下文缓存
    /// 4. 设置初始的上下文参数（如最大 token 数等）
    ///
    /// 创建的对话具有空的消息历史和上下文缓存。
    pub async fn create_conversation(
        &self,
        model_name: &str,
        user_id: Option<String>,
        title: Option<String>,
    ) -> MemoryResult<String> {
        let conv_id = Uuid::new_v4().to_string();
        let conversation = StoredConversation {
            id: conv_id.clone(),
            user_id,
            title,
            model_name: model_name.to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            message_count: 0,
            total_tokens: 0,
            summary: None,
            last_summary_sequence: None,
            system_message: None,
            system_message_hash: None,
            system_message_updated_at: None,
        };

        self.store.create_conversation(&conversation).await?;

        // 初始化上下文缓存
        let context = ContextMemory {
            conversation_id: conv_id.clone(),
            working_messages: Vec::new(),
            summary: None,
            total_tokens: 0,
            max_context_tokens: self.max_context_tokens(),
        };

        self.context_cache
            .lock()
            .await
            .insert(conv_id.clone(), context);

        Ok(conv_id)
    }

    /// 获取或创建用户对话
    ///
    /// # 参数
    /// * `user_id` - 用户的唯一标识符
    /// * `model_name` - 模型名称（用于创建新对话时设置，但不影响查找逻辑）
    ///
    /// # 返回值
    /// * `MemoryResult<String>` - 成功时返回对话ID（现有的或新创建的）
    ///
    /// # 说明
    /// 此方法实现用户对话的全局持久化管理：
    /// 1. 首先尝试找到用户的任何对话（不区分模型）
    /// 2. 如果找到，直接复用该对话ID并确保其在缓存中
    /// 3. 如果没有找到，为用户创建新对话
    /// 4. 同一个用户无论使用什么模型都会复用同一个对话
    pub async fn get_or_create_user_conversation(
        &self,
        user_id: &str,
        model_name: &str,
    ) -> MemoryResult<String> {
        // 尝试获取用户的任何对话（不区分模型）
        if let Some(recent_conv) = self
            .store
            .get_recent_conversation_by_user(user_id, None)
            .await?
        {
            // 对话存在，直接复用，确保其在缓存中
            self.ensure_conversation_in_cache(&recent_conv.id).await?;
            return Ok(recent_conv.id);
        }

        // 没有找到对话，创建新的
        self.create_conversation(model_name, Some(user_id.to_string()), None)
            .await
    }

    /// 确保对话在缓存中
    ///
    /// # 参数
    /// * `conv_id` - 对话ID
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回()
    ///
    /// # 说明
    /// 如果对话不在缓存中，从数据库加载并初始化缓存
    async fn ensure_conversation_in_cache(&self, conv_id: &str) -> MemoryResult<()> {
        let mut cache = self.context_cache.lock().await;

        if !cache.contains_key(conv_id) {
            // 对话不在缓存中，需要从数据库加载
            let conversation = self.store.get_conversation(conv_id).await?;

            // 加载最近的消息到工作上下文
            let recent_messages = self
                .store
                .get_recent_messages(conv_id, self.max_working_messages())
                .await?;

            let context = ContextMemory {
                conversation_id: conv_id.to_string(),
                working_messages: recent_messages,
                summary: conversation.summary,
                total_tokens: 0, // 这里可以根据需要计算实际token数
                max_context_tokens: self.max_context_tokens(),
            };

            cache.insert(conv_id.to_string(), context);
        }

        Ok(())
    }

    /// 添加用户消息到对话中
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    /// * `content` - 用户消息的文本内容
    ///
    /// # 返回值
    /// * `MemoryResult<MessageResult>` - 成功时返回消息结果和 summarization 状态，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法会：
    /// 1. 自动分配消息序列号
    /// 2. 生成唯一的消息 ID
    /// 3. 将消息完整存储到数据库
    /// 4. 更新对话的工作上下文缓存
    /// 5. 如果上下文过长，触发自动摘要和截断
    /// 6. 返回存储的消息以及是否触发了 summarization 的信息
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    pub async fn add_user_message(
        &self,
        conv_id: &str,
        content: String,
    ) -> MemoryResult<MessageResult> {
        let sequence = self.store.get_next_sequence(conv_id).await?;
        let message = StoredMessage {
            id: Uuid::new_v4().to_string(),
            conversation_id: conv_id.to_string(),
            role: MessageRole::User,
            content,
            timestamp: Utc::now(),
            sequence,
            tokens: None,
            tool_calls: Vec::new(),
        };

        // 第一层：完整存储
        self.store.store_message(&message).await?;

        // 第二层：更新工作上下文
        let summarization_status = self
            .update_working_context(conv_id, message.clone())
            .await?;

        if summarization_status.triggered {
            dual_info!(
                "User message addition triggered summarization for conversation {}: {}",
                conv_id,
                summarization_status
                    .trigger_reason
                    .as_deref()
                    .unwrap_or("unknown reason")
            );
        }

        Ok(MessageResult::new(message, summarization_status))
    }

    /// 添加助手消息到对话中
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    /// * `content` - 助手回复的文本内容
    /// * `tool_calls` - 助手在此回复中使用的工具调用列表
    ///
    /// # 返回值
    /// * `MemoryResult<MessageResult>` - 成功时返回消息结果和 summarization 状态，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法处理 AI 助手的回复消息，包括可能的工具调用信息。
    /// 工具调用会被完整保存，包括调用参数、执行结果和状态信息。
    /// 与用户消息类似，会触发上下文管理和可能的摘要操作。
    /// 返回的 `MessageResult` 包含存储的消息信息以及是否触发了 summarization。
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    pub async fn add_assistant_message(
        &self,
        conv_id: &str,
        content: &str,
        tool_calls: Vec<StoredToolCall>,
    ) -> MemoryResult<MessageResult> {
        let sequence = self.store.get_next_sequence(conv_id).await?;
        let message = StoredMessage {
            id: Uuid::new_v4().to_string(),
            conversation_id: conv_id.to_string(),
            role: MessageRole::Assistant,
            content: content.to_string(),
            timestamp: Utc::now(),
            sequence,
            tokens: None,
            tool_calls,
        };

        // 第一层：完整存储
        self.store.store_message(&message).await?;

        // 第二层：更新工作上下文
        let summarization_status = self
            .update_working_context(conv_id, message.clone())
            .await?;

        Ok(MessageResult::new(message, summarization_status))
    }

    /// 添加或更新对话的系统消息
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    /// * `system_message` - 系统消息内容
    ///
    /// # 返回值
    /// * `MemoryResult<bool>` - 成功时返回 true 表示系统消息被更新，false 表示内容未变化
    ///
    /// # 说明
    /// 此方法会检查系统消息是否发生变化（通过内容哈希比较），
    /// 只有在内容确实发生变化时才会更新数据库，避免不必要的写操作。
    pub async fn set_system_message(
        &self,
        conv_id: &str,
        system_message: &str,
    ) -> MemoryResult<bool> {
        // 获取当前对话信息
        let conversation = self.store.get_conversation(conv_id).await?;

        // 计算新消息的哈希
        let new_hash = format!("{:x}", md5::compute(system_message.as_bytes()));

        // 检查是否需要更新
        let needs_update = match &conversation.system_message_hash {
            Some(existing_hash) => existing_hash != &new_hash,
            None => true, // 如果之前没有系统消息，需要更新
        };

        if needs_update {
            self.store
                .update_system_message(conv_id, Some(system_message))
                .await?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// 获取对话的系统消息
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    ///
    /// # 返回值
    /// * `MemoryResult<Option<String>>` - 成功时返回系统消息内容，如果不存在则返回 None
    #[allow(dead_code)]
    pub async fn get_system_message(&self, conv_id: &str) -> MemoryResult<Option<String>> {
        let conversation = self.store.get_conversation(conv_id).await?;
        Ok(conversation.system_message)
    }

    /// 清除对话的系统消息
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    #[allow(dead_code)]
    pub async fn clear_system_message(&self, conv_id: &str) -> MemoryResult<()> {
        self.store.update_system_message(conv_id, None).await
    }

    /// 获取用于模型推理的上下文消息
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<ModelMessage>>` - 成功时返回格式化的消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法返回适合直接传递给 LLM API 的消息格式。返回的消息包括：
    /// 1. 系统消息（如果存在对话摘要）
    /// 2. 工作上下文中的消息（已转换为模型格式）
    /// 3. 工具调用信息（转换为标准格式）
    ///
    /// 这是获取当前对话上下文的主要接口，会自动处理摘要和工具调用的格式转换。
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    #[allow(dead_code)]
    pub async fn get_model_context(&self, conv_id: &str) -> MemoryResult<Vec<ModelMessage>> {
        let cache = self.context_cache.lock().await;
        let context = cache
            .get(conv_id)
            .ok_or_else(|| MemoryError::ConversationNotFound(conv_id.to_string()))?;

        // 获取对话信息以检查是否有存储的系统消息
        let conversation = self.store.get_conversation(conv_id).await?;

        let mut model_messages = Vec::new();

        // 合并系统消息和摘要到单个系统消息中
        let mut system_content_parts = Vec::new();

        // 首先添加存储的系统消息（如果存在）
        if let Some(system_message) = &conversation.system_message {
            system_content_parts.push(system_message.clone());
        }

        // 然后添加对话摘要（如果存在）
        if let Some(summary) = &context.summary {
            system_content_parts.push(format!("Previous conversation summary: {summary}"));
        }

        // 如果有任何系统内容，创建单个系统消息
        if !system_content_parts.is_empty() {
            model_messages.push(ModelMessage {
                role: ModelRole::System,
                content: system_content_parts.join("\n\n"),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // 转换工作消息为模型格式
        for stored_msg in &context.working_messages {
            // 处理Assistant消息的工具调用
            let tool_calls = if !stored_msg.tool_calls.is_empty() {
                Some(self.convert_to_model_tool_calls(&stored_msg.tool_calls))
            } else {
                None
            };

            // 添加Assistant消息（包含工具调用请求，但不含结果）
            model_messages.push(ModelMessage {
                role: stored_msg.role.into(),
                content: stored_msg.content.clone(),
                tool_calls,
                tool_call_id: None,
            });

            // 为每个有执行结果的工具调用生成独立的tool消息
            for tool_call in &stored_msg.tool_calls {
                if let Some(result) = &tool_call.result {
                    let tool_result_content = if result.success {
                        // 成功的工具调用：返回实际结果
                        match &result.content {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }
                    } else {
                        // 失败的工具调用：返回错误信息
                        format!(
                            "Tool execution failed: {}",
                            result.error.as_deref().unwrap_or("Unknown error")
                        )
                    };

                    model_messages.push(ModelMessage {
                        role: ModelRole::Tool,
                        content: tool_result_content,
                        tool_calls: None,
                        tool_call_id: Some(tool_call.id.clone()),
                    });
                }
            }
        }

        Ok(model_messages)
    }

    /// 判断指定消息是否应该触发 summarization 检查
    ///
    /// # 参数
    /// * `message` - 要检查的消息
    ///
    /// # 返回值
    /// * `bool` - 如果消息应该触发 summarization 检查则返回 true
    ///
    /// # 触发条件
    /// * 用户消息：总是可能触发
    /// * 助手消息：只有当包含带有结果的工具调用时才触发
    /// * 其他消息类型：不触发
    fn should_trigger_summarization(&self, message: &StoredMessage) -> bool {
        match message.role {
            MessageRole::User => {
                dual_debug!("User message can trigger summarization");
                true
            }
            MessageRole::Assistant => {
                let has_tool_results = message.tool_calls.iter().any(|tc| tc.result.is_some());
                dual_debug!(
                    "Assistant message trigger check: has_tool_results={}, tool_calls_count={}",
                    has_tool_results,
                    message.tool_calls.len()
                );
                has_tool_results
            }
            _ => {
                dual_debug!(
                    "Message role {:?} does not trigger summarization",
                    message.role
                );
                false
            }
        }
    }

    async fn update_working_context(
        &self,
        conv_id: &str,
        new_message: StoredMessage,
    ) -> MemoryResult<SummarizationStatus> {
        let mut cache = self.context_cache.lock().await;
        let context = cache
            .get_mut(conv_id)
            .ok_or_else(|| MemoryError::ConversationNotFound(conv_id.to_string()))?;

        // 检查是否应该触发 summarization
        let should_trigger = self.should_trigger_summarization(&new_message);

        context.working_messages.push(new_message);
        context.total_tokens = self.calculate_total_tokens(&context.working_messages);

        // 只有在特定消息类型且超过限制时才触发 summarization
        if should_trigger
            && (context.total_tokens > context.max_context_tokens
                || context.working_messages.len() > self.max_working_messages())
        {
            let trigger_reason = if context.total_tokens > context.max_context_tokens {
                format!(
                    "Token limit exceeded: {} > {}",
                    context.total_tokens, context.max_context_tokens
                )
            } else {
                format!(
                    "Message count limit exceeded: {} > {}",
                    context.working_messages.len(),
                    self.max_working_messages()
                )
            };

            dual_info!(
                "Summarization triggered for conversation {}: {}",
                conv_id,
                trigger_reason
            );
            drop(cache); // 释放锁，避免在异步操作中持有
            self.truncate_and_summarize(conv_id, trigger_reason).await
        } else {
            if !should_trigger {
                dual_debug!(
                    "Summarization not triggered for conversation {}: message type does not qualify for summarization",
                    conv_id
                );
            } else {
                dual_debug!(
                    "Summarization not triggered for conversation {}: limits not exceeded (tokens: {}/{}, messages: {}/{})",
                    conv_id,
                    context.total_tokens,
                    context.max_context_tokens,
                    context.working_messages.len(),
                    self.max_working_messages()
                );
            }
            Ok(SummarizationStatus::not_triggered())
        }
    }

    async fn truncate_and_summarize(
        &self,
        conv_id: &str,
        trigger_reason: String,
    ) -> MemoryResult<SummarizationStatus> {
        if !self.enable_summarization() {
            dual_debug!(
                "Summarization is disabled, skipping for conversation {}",
                conv_id
            );
            return Ok(SummarizationStatus::not_triggered());
        }

        dual_info!(
            "Starting summarization process for conversation {}",
            conv_id
        );

        let mut cache = self.context_cache.lock().await;
        let context = cache
            .get_mut(conv_id)
            .ok_or_else(|| MemoryError::ConversationNotFound(conv_id.to_string()))?;

        // 计算要保留的消息数量
        let keep_count = self.calculate_keep_count(&context.working_messages);

        if keep_count < context.working_messages.len() {
            let to_summarize: Vec<StoredMessage> = context
                .working_messages
                .drain(0..(context.working_messages.len() - keep_count))
                .collect();

            if !to_summarize.is_empty() {
                // 获取现有摘要用于增量摘要生成
                let existing_summary = context.summary.clone();
                let summarize_count = to_summarize.len();

                // 对于完整历史摘要策略，需要获取所有需要摘要的历史消息
                let full_history_messages = if matches!(
                    self.config.summarization_strategy,
                    crate::config::SummarizationStrategy::FullHistory
                ) {
                    // 从数据库获取所有历史消息用于完整历史摘要
                    Some(
                        self.get_all_historical_messages_for_summary(conv_id, &to_summarize)
                            .await?,
                    )
                } else {
                    None
                };

                drop(cache); // 释放锁进行异步操作

                // 生成摘要
                let new_summary = self
                    .summarizer
                    .summarize_stored_messages(
                        &to_summarize,
                        existing_summary.as_deref(),
                        full_history_messages.as_deref(),
                    )
                    .await?;

                // 重新获取锁并更新上下文
                let mut cache = self.context_cache.lock().await;
                let context = cache.get_mut(conv_id).unwrap();
                let kept_count = context.working_messages.len();
                context.summary = Some(new_summary.clone());
                context.total_tokens = self.calculate_total_tokens(&context.working_messages);

                dual_info!(
                    "Summarization completed for conversation {}: {} messages summarized, {} messages kept, new summary length: {} chars",
                    conv_id,
                    summarize_count,
                    kept_count,
                    new_summary.len()
                );
                dual_debug!("Updated summary for conversation {conv_id}: {new_summary}");

                // 更新数据库中的摘要
                let last_sequence = to_summarize.last().map(|m| m.sequence);
                drop(cache);
                self.store
                    .update_conversation_summary(conv_id, &new_summary, last_sequence)
                    .await?;

                return Ok(SummarizationStatus::triggered(
                    summarize_count,
                    kept_count,
                    new_summary.len(),
                    trigger_reason,
                ));
            }
        }

        dual_info!(
            "Truncation completed without summarization for conversation {}: {} messages kept",
            conv_id,
            context.working_messages.len()
        );

        Ok(SummarizationStatus::not_triggered())
    }

    /// 计算在摘要截断时应该保留的消息数量
    ///
    /// # 参数
    /// * `messages` - 当前工作上下文中的消息列表
    ///
    /// # 返回值
    /// * `usize` - 应该保留的消息数量
    ///
    /// # 算法说明
    /// 此方法采用多层决策算法来确定最优的消息保留数量：
    ///
    /// ## 第一层：基于 Token 的动态计算
    /// - 计算目标保留的 token 数量：`max_context_tokens * (1 - summary_trigger_ratio)`
    /// - 默认保留20%的上下文空间给工作消息（80%触发摘要时）
    /// - 从最新消息开始反向累计，直到达到 token 或数量限制
    ///
    /// ## 第二层：配置最小值保证
    /// - 确保至少保留配置的最小消息数（`summarize_threshold/2`）
    /// - 取较大值：max(token_based_count, config_min_count)
    ///
    /// ## 第三层：消息对完整性调整
    /// - 调用 `adjust_for_message_pairs` 确保不会拆散 user-assistant 消息对
    /// - 如果分割点会打断对话对，会适当调整保留数量
    ///
    /// # 设计目标
    /// - **智能平衡**：在节省上下文空间和保持信息完整性之间找到平衡
    /// - **配置驱动**：支持通过配置参数调整保留策略
    /// - **对话连贯**：确保保留的消息在语义上完整
    /// - **性能优化**：避免过度保留导致的上下文膨胀
    fn calculate_keep_count(&self, messages: &[StoredMessage]) -> usize {
        let target_tokens =
            (self.max_context_tokens() as f32 * (1.0 - self.summary_trigger_ratio())) as usize;
        let mut current_tokens = 0;
        let mut keep_count = 0;

        // 从后往前计算，保留最近的消息
        for message in messages.iter().rev() {
            let msg_tokens = self.estimate_message_tokens(message);
            if current_tokens + msg_tokens <= target_tokens
                && keep_count < self.max_working_messages()
            {
                current_tokens += msg_tokens;
                keep_count += 1;
            } else {
                break;
            }
        }

        // 至少保留配置的最小消息数
        let min_keep = self.keep_recent_messages().min(messages.len());
        let ideal_keep = keep_count.max(min_keep);

        // 确保消息对的完整性：调整保留数量以避免拆散 user-assistant 对
        self.adjust_for_message_pairs(messages, ideal_keep)
    }

    /// 调整保留数量以确保 user-assistant 消息对的完整性
    fn adjust_for_message_pairs(&self, messages: &[StoredMessage], mut keep_count: usize) -> usize {
        if keep_count >= messages.len() {
            return messages.len();
        }

        let split_index = messages.len() - keep_count;

        // 检查分割点是否会拆散消息对
        if split_index > 0 && split_index < messages.len() {
            let prev_msg = &messages[split_index - 1];
            let curr_msg = &messages[split_index];

            // 如果前一条是 User 消息，当前是 Assistant 消息，说明会拆散一对
            if matches!(prev_msg.role, MessageRole::User)
                && matches!(curr_msg.role, MessageRole::Assistant)
            {
                // 选择策略：优先完整保留消息对
                // 选项1：多保留一条（包含完整的user-assistant对）
                if keep_count < messages.len() {
                    keep_count += 1;
                }
                // 选项2：少保留一条（避免拆散，将user消息也放入摘要）
                // keep_count = keep_count.saturating_sub(1);
            }
        }

        keep_count
    }

    fn calculate_total_tokens(&self, messages: &[StoredMessage]) -> usize {
        messages
            .iter()
            .map(|m| self.estimate_message_tokens(m))
            .sum()
    }

    fn estimate_message_tokens(&self, message: &StoredMessage) -> usize {
        // 简化的token估算
        let content_tokens = message.content.len() / 4;
        let tool_tokens = message.tool_calls.len() * 100; // 每个工具调用约100 tokens
        content_tokens + tool_tokens
    }

    fn convert_to_model_tool_calls(&self, tool_calls: &[StoredToolCall]) -> Vec<ModelToolCall> {
        tool_calls
            .iter()
            .map(|tc| ModelToolCall {
                id: tc.id.clone(),
                ty: "function".to_string(),
                function: ModelToolFunction {
                    name: tc.name.clone(),
                    arguments: tc.arguments.to_string(),
                },
            })
            .collect()
    }

    /// 获取指定对话的元数据（metadata）
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<StoredConversation>` - 成功时返回对话的元数据对象，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回对话的元数据信息，**不包含**具体的聊天消息内容。元数据包括：
    /// - 对话基本信息：ID、标题、所属用户、使用的模型
    /// - 时间信息：创建时间、最后更新时间
    /// - 统计信息：消息总数、token 总数
    /// - 摘要信息：对话摘要、最后摘要位置
    ///
    /// 这是一个轻量级查询操作，适用于对话列表显示、统计分析等场景。
    /// 如需获取完整的聊天消息内容，请使用 `get_full_history(conv_id, include_system_message)` 方法。
    ///
    /// 这是一个直接的数据库查询操作，不涉及上下文缓存。
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    #[allow(dead_code)]
    pub async fn get_conversation(&self, conv_id: &str) -> MemoryResult<StoredConversation> {
        self.store.get_conversation(conv_id).await
    }

    /// 获取对话列表摘要
    ///
    /// # 参数
    /// * `limit` - 返回的最大对话数量，None 表示使用默认限制
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<ConversationSummary>>` - 成功时返回对话摘要列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回按最后更新时间降序排列的对话摘要列表，用于在 UI 中显示对话历史。
    /// 每个摘要包含对话的基本信息但不包含详细的消息内容，适合快速浏览。
    #[allow(dead_code)]
    pub async fn list_conversations(
        &self,
        limit: Option<usize>,
    ) -> MemoryResult<Vec<ConversationSummary>> {
        self.store.list_conversations(limit).await
    }

    /// 获取指定用户的对话列表摘要
    ///
    /// # 参数
    /// * `user_id` - 用户的唯一标识符
    /// * `limit` - 返回的最大对话数量，None 表示使用默认限制
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<ConversationSummary>>` - 成功时返回该用户的对话摘要列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回指定用户按最后更新时间降序排列的对话摘要列表，用于在 UI 中显示用户的对话历史。
    /// 每个摘要包含对话的基本信息但不包含详细的消息内容，适合快速浏览。
    ///
    /// 注意：虽然当前系统设计为用户与对话的1:1关系，但此方法支持一个用户拥有多个对话的场景。
    pub async fn list_user_conversations(
        &self,
        user_id: &str,
        limit: Option<usize>,
    ) -> MemoryResult<Vec<ConversationSummary>> {
        self.store.list_conversations_by_user(user_id, limit).await
    }

    /// 获取对话的当前工作消息
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 成功时返回工作消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回对话的当前工作上下文中的消息，按序列号升序排列。这些是当前用于模型推理的消息，
    /// 不包括已被摘要化的历史消息。与 `get_full_history` 不同，此方法只返回活跃的工作消息。
    ///
    /// 工作消息是内存管理器当前维护的消息集合，通常是最近的N条消息，具体数量由配置决定。
    /// 当消息历史过长时，旧消息会被摘要化，只保留最近的工作消息用于模型推理。
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    pub async fn get_working_messages(&self, conv_id: &str) -> MemoryResult<Vec<StoredMessage>> {
        let cache = self.context_cache.lock().await;
        let context = cache
            .get(conv_id)
            .ok_or_else(|| MemoryError::ConversationNotFound(conv_id.to_string()))?;

        Ok(context.working_messages.clone())
    }

    /// 获取对话的完整消息历史
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    /// * `include_system_message` - 是否在返回结果的开头包含系统消息
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 成功时返回完整的消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回对话中的所有消息，按序列号升序排列。与 `get_model_context` 不同，
    /// 此方法返回完整的历史记录而不是当前的工作上下文。
    /// 适用于导出对话、审计或分析等场景。
    ///
    /// ## 系统消息处理
    /// * 当 `include_system_message` 为 `true` 时：
    ///   - 如果对话有系统消息，会在返回列表的开头插入一个系统消息
    ///   - 系统消息的序列号为 0，时间戳为对话创建时间
    /// * 当 `include_system_message` 为 `false` 时：
    ///   - 只返回用户、助手和工具消息，不包含系统消息
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    pub async fn get_full_history(
        &self,
        conv_id: &str,
        include_system_message: bool,
    ) -> MemoryResult<Vec<StoredMessage>> {
        let mut messages = self.store.get_full_history(conv_id).await?;

        if include_system_message {
            // 获取对话信息以检查是否有系统消息
            let conversation = self.store.get_conversation(conv_id).await?;

            if let Some(system_message) = conversation.system_message {
                // 创建系统消息对象，插入到列表开头
                let system_msg = StoredMessage {
                    id: format!("system-{conv_id}"),
                    conversation_id: conv_id.to_string(),
                    role: MessageRole::System,
                    content: system_message,
                    timestamp: conversation.created_at,
                    sequence: 0, // 系统消息序列号为 0
                    tokens: None,
                    tool_calls: Vec::new(),
                };

                messages.insert(0, system_msg);
            }
        }

        Ok(messages)
    }

    /// 通过用户ID获取完整聊天历史
    ///
    /// # 参数
    /// * `user_id` - 用户的唯一标识符
    /// * `include_system_message` - 是否在返回结果的开头包含系统消息
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 成功时返回完整的消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 基于用户ID与对话ID的1:1关系，此方法会：
    /// 1. 首先查找用户对应的对话（不区分模型）
    /// 2. 如果找到对话，返回该对话的完整消息历史
    /// 3. 如果用户没有任何对话，返回空的消息列表
    ///
    /// 这是一个便捷方法，避免了先获取对话ID再获取历史的两步操作。
    /// 返回的消息按序列号升序排列，包含完整的对话历史记录。
    ///
    /// ## 系统消息处理
    /// * 当 `include_system_message` 为 `true` 时：
    ///   - 如果对话有系统消息，会在返回列表的开头插入一个系统消息
    /// * 当 `include_system_message` 为 `false` 时：
    ///   - 只返回用户、助手和工具消息，不包含系统消息
    ///
    /// # 错误
    /// * `MemoryError::DatabaseError` - 当数据库查询失败时
    pub async fn get_user_full_history(
        &self,
        user_id: &str,
        include_system_message: bool,
    ) -> MemoryResult<Vec<StoredMessage>> {
        // 1. 获取用户的对话ID
        if let Some(conv) = self
            .store
            .get_recent_conversation_by_user(user_id, None)
            .await?
        {
            // 2. 获取完整聊天记录
            self.get_full_history(&conv.id, include_system_message)
                .await
        } else {
            Ok(Vec::new()) // 用户没有对话历史
        }
    }

    /// 删除指定的对话及其所有相关数据
    ///
    /// # 参数
    /// * `conv_id` - 要删除的对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法会完全删除对话的所有数据，包括：
    /// 1. 内存缓存中的上下文数据
    /// 2. 数据库中的对话记录和所有消息
    ///
    /// 注意：此操作不可逆，请谨慎使用。由于设置了外键约束的级联删除，
    /// 删除对话时会自动删除该对话下的所有消息。
    #[allow(dead_code)]
    pub async fn delete_conversation(&self, conv_id: &str) -> MemoryResult<()> {
        // 从缓存中移除
        self.context_cache.lock().await.remove(conv_id);

        // 从数据库中删除
        self.store.delete_conversation(conv_id).await
    }

    /// 获取用于完整历史摘要的所有历史消息
    ///
    /// # 参数
    /// * `conv_id` - 对话 ID
    /// * `current_to_summarize` - 当前需要摘要的消息列表
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 包含所有需要摘要的历史消息
    ///
    /// # 说明
    /// 对于完整历史摘要策略，我们需要获取：
    /// 1. `current_to_summarize` 中的所有消息
    /// 2. 数据库中序列号小于 `current_to_summarize` 最小序列号的历史消息
    ///
    /// 例如：
    /// - 如果 `current_to_summarize` 包含序列号 [3,4] 的消息
    /// - 数据库中包含序列号 [1,2,3,4,5,6,...] 的消息
    /// - 则返回序列号 [1,2,3,4] 的消息用于摘要
    ///
    /// 这样可以避免重复包含消息，确保摘要包含完整的历史上下文。
    async fn get_all_historical_messages_for_summary(
        &self,
        conv_id: &str,
        current_to_summarize: &[StoredMessage],
    ) -> MemoryResult<Vec<StoredMessage>> {
        // 从数据库获取所有历史消息（不包含系统消息，因为摘要不需要）
        let historical_messages = self.store.get_full_history(conv_id).await?;

        // 找到 current_to_summarize 中的最小序列号
        let min_current_sequence = current_to_summarize
            .iter()
            .map(|msg| msg.sequence)
            .min()
            .unwrap_or(i64::MAX);

        // 只保留序列号小于 current_to_summarize 最小序列号的历史消息
        let mut filtered_historical: Vec<StoredMessage> = historical_messages
            .into_iter()
            .filter(|msg| msg.sequence < min_current_sequence)
            .collect();

        // 将过滤后的历史消息和当前要摘要的消息合并
        filtered_historical.extend_from_slice(current_to_summarize);

        // 按序列号排序，确保消息顺序正确
        filtered_historical.sort_by_key(|msg| msg.sequence);

        dual_debug!(
            "All messages for summary (filtered):\n{}",
            serde_json::to_string_pretty(&filtered_historical).unwrap()
        );

        Ok(filtered_historical)
    }

    /// 获取内存系统的统计信息
    ///
    /// # 返回值
    /// * `MemoryResult<MemoryStats>` - 成功时返回统计信息，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回整个记忆系统的统计数据，包括：
    /// - 对话总数和消息总数
    /// - Token 使用统计
    /// - 数据库大小等信息
    ///
    /// 这些统计信息可用于监控系统使用情况、容量规划和性能分析。
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> MemoryResult<MemoryStats> {
        self.store.get_stats().await
    }
}
