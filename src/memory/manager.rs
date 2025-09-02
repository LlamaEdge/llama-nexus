use std::collections::HashMap;

use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{
    config::MemoryConfig,
    dual_debug, dual_info,
    memory::{store::MessageStore, summarizer::MessageSummarizer, types::*},
};

/// Complete chat memory manager
///
/// Provides complete lifecycle management for conversations, including message storage, context management, automatic summarization and other features.
/// This struct is the core of the entire memory system, responsible for coordinating the storage layer, summarizer, and configuration management.
///
/// # Features
/// * Hierarchical storage: Complete database storage + working context cache
/// * Intelligent summarization: Automatically generates summaries when context becomes too long to save space
/// * Configuration-driven: Supports customizing behavior parameters through configuration files
/// * Concurrency-safe: Uses async locks to ensure thread safety
pub struct CompleteChatMemory {
    /// Underlying message storage, responsible for persisting data to SQLite database
    ///
    /// Provides complete CRUD operations, including message storage, conversation management, statistical queries, etc.
    /// All conversation and message data is persisted through this component.
    store: MessageStore,

    /// Context cache, stores working context for each conversation
    ///
    /// Key: Conversation ID (String)
    /// Value: Conversation's context memory (ContextMemory)
    ///
    /// Uses async mutex lock to ensure concurrency safety, caches working message sets for currently active conversations,
    /// avoiding loading complete history from database every time. Auto-summary and truncation are triggered when context becomes too long.
    context_cache: Mutex<HashMap<String, ContextMemory>>,

    /// Message summarizer for compressing long conversation history
    ///
    /// When conversation context exceeds configured length limit, this component is used to
    /// compress old messages into summary text to save context space while preserving important information.
    /// Supports incremental summarization, can generate updated summaries based on existing summaries and new messages.
    summarizer: MessageSummarizer,

    /// Memory system configuration parameters
    ///
    /// Contains various configurable behavior parameters, such as:
    /// - Database file path
    /// - Context window size limit
    /// - Auto-summary trigger conditions
    /// - Maximum stored message count, etc.
    ///
    /// Loaded from configuration file, supports runtime customization of system behavior.
    config: MemoryConfig,
}

impl CompleteChatMemory {
    /// Create a new complete chat memory manager instance
    ///
    /// # Parameters
    /// * `config` - Memory system configuration, including database path, context window size and other settings
    ///
    /// # Returns
    /// * `MemoryResult<Self>` - Returns manager instance on success, MemoryError on failure
    ///
    /// # Description
    /// This method will:
    /// 1. Initialize the underlying message storage (MessageStore)
    /// 2. Create message summarizer (MessageSummarizer)
    /// 3. Initialize context cache
    /// 4. Apply configuration parameters
    ///
    /// # Errors
    /// * `MemoryError::DatabaseError` - When database connection or initialization fails
    pub async fn new(config: MemoryConfig) -> MemoryResult<Self> {
        // Initialize message storage
        let store = MessageStore::new(&config.database_path).await?;

        // Create message summarizer
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

    // Configuration mapping helper methods
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
        // Trigger summarization when message count exceeds threshold, set to ratio of 0.8
        0.8
    }

    fn keep_recent_messages(&self) -> usize {
        // Keep recent message count, set to half of threshold
        (self.config.summarize_threshold / 2) as usize
    }

    /// Create a new conversation
    ///
    /// # Parameters
    /// * `model_name` - Model name used for the conversation
    /// * `user_id` - Optional user ID
    /// * `title` - Optional conversation title
    ///
    /// # Returns
    /// * `MemoryResult<String>` - Returns newly created conversation ID on success, MemoryError on failure
    ///
    /// # Description
    /// This method will:
    /// 1. Generate unique conversation ID
    /// 2. Create conversation record in database
    /// 3. Initialize conversation's context cache
    /// 4. Set initial context parameters (such as max token count, etc.)
    ///
    /// The created conversation has empty message history and context cache.
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

        // Initialize context cache
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

    /// Get or create user conversation
    ///
    /// # Parameters
    /// * `user_id` - User's unique identifier
    /// * `model_name` - Model name (used when creating new conversation, but doesn't affect lookup logic)
    ///
    /// # Returns
    /// * `MemoryResult<String>` - Returns conversation ID (existing or newly created) on success
    ///
    /// # Description
    /// This method implements global persistent management of user conversations:
    /// 1. First try to find any conversation for the user (regardless of model)
    /// 2. If found, directly reuse that conversation ID and ensure it's in cache
    /// 3. If not found, create new conversation for the user
    /// 4. Same user will reuse the same conversation regardless of which model is used
    pub async fn get_or_create_user_conversation(
        &self,
        user_id: &str,
        model_name: &str,
    ) -> MemoryResult<String> {
        // Try to get any conversation for the user (regardless of model)
        if let Some(recent_conv) = self
            .store
            .get_recent_conversation_by_user(user_id, None)
            .await?
        {
            // Conversation exists, directly reuse, ensure it's in cache
            self.ensure_conversation_in_cache(&recent_conv.id).await?;
            return Ok(recent_conv.id);
        }

        // No conversation found, create new one
        self.create_conversation(model_name, Some(user_id.to_string()), None)
            .await
    }

    /// Ensure conversation is in cache
    ///
    /// # Parameters
    /// * `conv_id` - Conversation ID
    ///
    /// # Returns
    /// * `MemoryResult<()>` - Returns () on success
    ///
    /// # Description
    /// If conversation is not in cache, load from database and initialize cache
    async fn ensure_conversation_in_cache(&self, conv_id: &str) -> MemoryResult<()> {
        let mut cache = self.context_cache.lock().await;

        if !cache.contains_key(conv_id) {
            // Conversation not in cache, need to load from database
            let conversation = self.store.get_conversation(conv_id).await?;

            // Load recent messages to working context
            let recent_messages = self
                .store
                .get_recent_messages(conv_id, self.max_working_messages())
                .await?;

            let context = ContextMemory {
                conversation_id: conv_id.to_string(),
                working_messages: recent_messages,
                summary: conversation.summary,
                total_tokens: 0, // Can calculate actual token count here if needed
                max_context_tokens: self.max_context_tokens(),
            };

            cache.insert(conv_id.to_string(), context);
        }

        Ok(())
    }

    /// Add user message to conversation
    ///
    /// # Parameters
    /// * `conv_id` - Target conversation ID
    /// * `content` - User message text content
    ///
    /// # Returns
    /// * `MemoryResult<MessageResult>` - Returns message result and summarization status on success, MemoryError on failure
    ///
    /// # Description
    /// This method will:
    /// 1. Automatically assign message sequence number
    /// 2. Generate unique message ID
    /// 3. Store message completely to database
    /// 4. Update conversation's working context cache
    /// 5. If context is too long, trigger automatic summarization and truncation
    /// 6. Return stored message and whether summarization was triggered
    ///
    /// # Errors
    /// * `MemoryError::ConversationNotFound` - When specified conversation doesn't exist
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

        // First layer: complete storage
        self.store.store_message(&message).await?;

        // Second layer: update working context
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

    /// Add assistant message to conversation
    ///
    /// # Parameters
    /// * `conv_id` - Target conversation ID
    /// * `content` - Assistant reply text content
    /// * `tool_calls` - List of tool calls used by assistant in this reply
    ///
    /// # Returns
    /// * `MemoryResult<MessageResult>` - Returns message result and summarization status on success, MemoryError on failure
    ///
    /// # Description
    /// This method handles AI assistant reply messages, including possible tool call information.
    /// Tool calls will be completely saved, including call parameters, execution results and status information.
    /// Similar to user messages, it will trigger context management and possible summarization operations.
    /// The returned `MessageResult` contains stored message information and whether summarization was triggered.
    ///
    /// # Errors
    /// * `MemoryError::ConversationNotFound` - When specified conversation doesn't exist
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

        // First layer: complete storage
        self.store.store_message(&message).await?;

        // Second layer: update working context
        let summarization_status = self
            .update_working_context(conv_id, message.clone())
            .await?;

        Ok(MessageResult::new(message, summarization_status))
    }

    /// Add or update conversation's system message
    ///
    /// # Parameters
    /// * `conv_id` - Target conversation ID
    /// * `system_message` - System message content
    ///
    /// # Returns
    /// * `MemoryResult<bool>` - Returns true if system message was updated, false if content unchanged on success
    ///
    /// # Description
    /// This method will check if system message has changed (by content hash comparison),
    /// only updating database when content actually changes, avoiding unnecessary write operations.
    pub async fn set_system_message(
        &self,
        conv_id: &str,
        system_message: &str,
    ) -> MemoryResult<bool> {
        // Get current conversation information
        let conversation = self.store.get_conversation(conv_id).await?;

        // Calculate hash of new message
        let new_hash = format!("{:x}", md5::compute(system_message.as_bytes()));

        // Check if update is needed
        let needs_update = match &conversation.system_message_hash {
            Some(existing_hash) => existing_hash != &new_hash,
            None => true, // If no previous system message, update is needed
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

    /// Get conversation's system message
    ///
    /// # Parameters
    /// * `conv_id` - Target conversation ID
    ///
    /// # Returns
    /// * `MemoryResult<Option<String>>` - Returns system message content on success, None if doesn't exist
    #[allow(dead_code)]
    pub async fn get_system_message(&self, conv_id: &str) -> MemoryResult<Option<String>> {
        let conversation = self.store.get_conversation(conv_id).await?;
        Ok(conversation.system_message)
    }

    /// Clear conversation's system message
    ///
    /// # Parameters
    /// * `conv_id` - Target conversation ID
    ///
    /// # Returns
    /// * `MemoryResult<()>` - Returns () on success, MemoryError on failure
    #[allow(dead_code)]
    pub async fn clear_system_message(&self, conv_id: &str) -> MemoryResult<()> {
        self.store.update_system_message(conv_id, None).await
    }

    /// Get context messages for model inference
    ///
    /// # Parameters
    /// * `conv_id` - Target conversation ID
    ///
    /// # Returns
    /// * `MemoryResult<Vec<ModelMessage>>` - Returns formatted message list on success, MemoryError on failure
    ///
    /// # Description
    /// This method returns message format suitable for direct passing to LLM API. Returned messages include:
    /// 1. System message (if conversation summary exists)
    /// 2. Messages in working context (converted to model format)
    /// 3. Tool call information (converted to standard format)
    ///
    /// This is the main interface for getting current conversation context, automatically handles summarization and tool call format conversion.
    ///
    /// # Errors
    /// * `MemoryError::ConversationNotFound` - When specified conversation doesn't exist
    #[allow(dead_code)]
    pub async fn get_model_context(&self, conv_id: &str) -> MemoryResult<Vec<ModelMessage>> {
        let cache = self.context_cache.lock().await;
        let context = cache
            .get(conv_id)
            .ok_or_else(|| MemoryError::ConversationNotFound(conv_id.to_string()))?;

        // Get conversation information to check if there's a stored system message
        let conversation = self.store.get_conversation(conv_id).await?;

        let mut model_messages = Vec::new();

        // Merge system message and summary into single system message
        let mut system_content_parts = Vec::new();

        // First add stored system message (if exists)
        if let Some(system_message) = &conversation.system_message {
            system_content_parts.push(system_message.clone());
        }

        // Then add conversation summary (if exists)
        if let Some(summary) = &context.summary {
            system_content_parts.push(format!("Previous conversation summary: {summary}"));
        }

        // If there's any system content, create single system message
        if !system_content_parts.is_empty() {
            model_messages.push(ModelMessage {
                role: ModelRole::System,
                content: system_content_parts.join("\n\n"),
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Convert working messages to model format
        for stored_msg in &context.working_messages {
            // Handle tool calls for Assistant messages
            let tool_calls = if !stored_msg.tool_calls.is_empty() {
                Some(self.convert_to_model_tool_calls(&stored_msg.tool_calls))
            } else {
                None
            };

            // Add Assistant message (contains tool call requests, but not results)
            model_messages.push(ModelMessage {
                role: stored_msg.role.into(),
                content: stored_msg.content.clone(),
                tool_calls,
                tool_call_id: None,
            });

            // Generate independent tool message for each tool call with execution result
            for tool_call in &stored_msg.tool_calls {
                if let Some(result) = &tool_call.result {
                    let tool_result_content = if result.success {
                        // Successful tool call: return actual result
                        match &result.content {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        }
                    } else {
                        // Failed tool call: return error information
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

    /// Determine if specified message should trigger summarization check
    ///
    /// # Parameters
    /// * `message` - Message to check
    ///
    /// # Returns
    /// * `bool` - Returns true if message should trigger summarization check
    ///
    /// # Trigger Conditions
    /// * User messages: Always may trigger
    /// * Assistant messages: Only trigger when containing tool calls with results
    /// * Other message types: Do not trigger
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

        // Check if should trigger summarization
        let should_trigger = self.should_trigger_summarization(&new_message);

        context.working_messages.push(new_message);
        context.total_tokens = self.calculate_total_tokens(&context.working_messages);

        // Only trigger summarization when specific message types exceed limits
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
            drop(cache); // Release lock to avoid holding during async operations
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

        // Calculate number of messages to keep
        let keep_count = self.calculate_keep_count(&context.working_messages);

        if keep_count < context.working_messages.len() {
            let to_summarize: Vec<StoredMessage> = context
                .working_messages
                .drain(0..(context.working_messages.len() - keep_count))
                .collect();

            if !to_summarize.is_empty() {
                // Get existing summary for incremental summary generation
                let existing_summary = context.summary.clone();
                let summarize_count = to_summarize.len();

                // For full history summary strategy, need to get all historical messages that need summarization
                let full_history_messages = if matches!(
                    self.config.summarization_strategy,
                    crate::config::SummarizationStrategy::FullHistory
                ) {
                    // Get all historical messages from database for full history summary
                    Some(
                        self.get_all_historical_messages_for_summary(conv_id, &to_summarize)
                            .await?,
                    )
                } else {
                    None
                };

                drop(cache); // Release lock for async operation

                // Generate summary
                let new_summary = self
                    .summarizer
                    .summarize_stored_messages(
                        &to_summarize,
                        existing_summary.as_deref(),
                        full_history_messages.as_deref(),
                    )
                    .await?;

                // Re-acquire lock and update context
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

                // Update summary in database
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

    /// Calculate the number of messages to keep during summary truncation
    ///
    /// # Parameters
    /// * `messages` - Message list in current working context
    ///
    /// # Returns
    /// * `usize` - Number of messages that should be kept
    ///
    /// # Algorithm Description
    /// This method uses a multi-layer decision algorithm to determine the optimal number of messages to keep:
    ///
    /// ## Layer 1: Token-based Dynamic Calculation
    /// - Calculate target token count to keep: `max_context_tokens * (1 - summary_trigger_ratio)`
    /// - By default keep 20% of context space for working messages (when 80% triggers summary)
    /// - Accumulate backwards from newest messages until reaching token or count limit
    ///
    /// ## Layer 2: Configuration Minimum Guarantee
    /// - Ensure at least the configured minimum message count is kept (`summarize_threshold/2`)
    /// - Take the larger value: max(token_based_count, config_min_count)
    ///
    /// ## Layer 3: Message Pair Integrity Adjustment
    /// - Call `adjust_for_message_pairs` to ensure user-assistant message pairs aren't split
    /// - If split point would break conversation pairs, adjust keep count appropriately
    ///
    /// # Design Goals
    /// - **Intelligent Balance**: Find balance between saving context space and maintaining information integrity
    /// - **Configuration-driven**: Support adjusting retention strategy through configuration parameters
    /// - **Conversation Coherence**: Ensure kept messages are semantically complete
    /// - **Performance Optimization**: Avoid context bloat from excessive retention
    fn calculate_keep_count(&self, messages: &[StoredMessage]) -> usize {
        let target_tokens =
            (self.max_context_tokens() as f32 * (1.0 - self.summary_trigger_ratio())) as usize;
        let mut current_tokens = 0;
        let mut keep_count = 0;

        // Calculate backwards from end, keeping most recent messages
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

        // Keep at least the configured minimum message count
        let min_keep = self.keep_recent_messages().min(messages.len());
        let ideal_keep = keep_count.max(min_keep);

        // Ensure message pair integrity: adjust keep count to avoid splitting user-assistant pairs
        self.adjust_for_message_pairs(messages, ideal_keep)
    }

    /// Adjust keep count to ensure user-assistant message pair integrity
    fn adjust_for_message_pairs(&self, messages: &[StoredMessage], mut keep_count: usize) -> usize {
        if keep_count >= messages.len() {
            return messages.len();
        }

        let split_index = messages.len() - keep_count;

        // Check if split point would break message pairs
        if split_index > 0 && split_index < messages.len() {
            let prev_msg = &messages[split_index - 1];
            let curr_msg = &messages[split_index];

            // If previous is User message and current is Assistant message, it would break a pair
            if matches!(prev_msg.role, MessageRole::User)
                && matches!(curr_msg.role, MessageRole::Assistant)
            {
                // Strategy choice: prioritize keeping complete message pairs
                // Option 1: Keep one more (include complete user-assistant pair)
                if keep_count < messages.len() {
                    keep_count += 1;
                }
                // Option 2: Keep one less (avoid breaking, put user message into summary too)
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
        // Simplified token estimation
        let content_tokens = message.content.len() / 4;
        let tool_tokens = message.tool_calls.len() * 100; // About 100 tokens per tool call
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

    /// Get metadata for specified conversation
    ///
    /// # Parameters
    /// * `conv_id` - Unique identifier of the conversation
    ///
    /// # Returns
    /// * `MemoryResult<StoredConversation>` - Returns conversation metadata object on success, MemoryError on failure
    ///
    /// # Description
    /// Returns conversation metadata information, **excluding** specific chat message content. Metadata includes:
    /// - Basic conversation info: ID, title, owner user, model used
    /// - Time information: creation time, last update time
    /// - Statistics: total message count, total token count
    /// - Summary information: conversation summary, last summary position
    ///
    /// This is a lightweight query operation, suitable for conversation list display, statistical analysis and similar scenarios.
    /// To get complete chat message content, please use `get_full_history(conv_id, include_system_message)` method.
    ///
    /// This is a direct database query operation that doesn't involve context cache.
    ///
    /// # Errors
    /// * `MemoryError::ConversationNotFound` - When specified conversation doesn't exist
    #[allow(dead_code)]
    pub async fn get_conversation(&self, conv_id: &str) -> MemoryResult<StoredConversation> {
        self.store.get_conversation(conv_id).await
    }

    /// Get conversation list summary
    ///
    /// # Parameters
    /// * `limit` - Maximum number of conversations to return, None means use default limit
    ///
    /// # Returns
    /// * `MemoryResult<Vec<ConversationSummary>>` - Returns conversation summary list on success, MemoryError on failure
    ///
    /// # Description
    /// Returns conversation summary list sorted by last update time in descending order, for displaying conversation history in UI.
    /// Each summary contains basic conversation information but not detailed message content, suitable for quick browsing.
    #[allow(dead_code)]
    pub async fn list_conversations(
        &self,
        limit: Option<usize>,
    ) -> MemoryResult<Vec<ConversationSummary>> {
        self.store.list_conversations(limit).await
    }

    /// Get conversation list summary for specified user
    ///
    /// # Parameters
    /// * `user_id` - User's unique identifier
    /// * `limit` - Maximum number of conversations to return, None means use default limit
    ///
    /// # Returns
    /// * `MemoryResult<Vec<ConversationSummary>>` - Returns conversation summary list for the user on success, MemoryError on failure
    ///
    /// # Description
    /// Returns conversation summary list for specified user sorted by last update time in descending order, for displaying user's conversation history in UI.
    /// Each summary contains basic conversation information but not detailed message content, suitable for quick browsing.
    ///
    /// Note: Although current system is designed for 1:1 relationship between user and conversation, this method supports scenarios where one user has multiple conversations.
    pub async fn list_user_conversations(
        &self,
        user_id: &str,
        limit: Option<usize>,
    ) -> MemoryResult<Vec<ConversationSummary>> {
        self.store.list_conversations_by_user(user_id, limit).await
    }

    /// Get current working messages for conversation
    ///
    /// # Parameters
    /// * `conv_id` - Unique identifier of the conversation
    ///
    /// # Returns
    /// * `MemoryResult<Vec<StoredMessage>>` - Returns working message list on success, MemoryError on failure
    ///
    /// # Description
    /// Returns messages in the conversation's current working context, sorted by sequence number in ascending order. These are the messages currently used for model inference,
    /// excluding historical messages that have been summarized. Unlike `get_full_history`, this method only returns active working messages.
    ///
    /// Working messages are the message set currently maintained by the memory manager, typically the most recent N messages, with specific count determined by configuration.
    /// When message history becomes too long, old messages are summarized, keeping only recent working messages for model inference.
    ///
    /// # Errors
    /// * `MemoryError::ConversationNotFound` - When specified conversation doesn't exist
    pub async fn get_working_messages(&self, conv_id: &str) -> MemoryResult<Vec<StoredMessage>> {
        let cache = self.context_cache.lock().await;
        let context = cache
            .get(conv_id)
            .ok_or_else(|| MemoryError::ConversationNotFound(conv_id.to_string()))?;

        Ok(context.working_messages.clone())
    }

    /// Get complete message history for conversation
    ///
    /// # Parameters
    /// * `conv_id` - Unique identifier of the conversation
    /// * `include_system_message` - Whether to include system message at the beginning of returned results
    ///
    /// # Returns
    /// * `MemoryResult<Vec<StoredMessage>>` - Returns complete message list on success, MemoryError on failure
    ///
    /// # Description
    /// Returns all messages in the conversation, sorted by sequence number in ascending order. Unlike `get_model_context`,
    /// this method returns complete historical records rather than current working context.
    /// Suitable for conversation export, auditing, analysis and similar scenarios.
    ///
    /// ## System Message Handling
    /// * When `include_system_message` is `true`:
    ///   - If conversation has system message, it will be inserted at the beginning of returned list
    ///   - System message sequence number is 0, timestamp is conversation creation time
    /// * When `include_system_message` is `false`:
    ///   - Only returns user, assistant and tool messages, excluding system messages
    ///
    /// # Errors
    /// * `MemoryError::ConversationNotFound` - When specified conversation doesn't exist
    pub async fn get_full_history(
        &self,
        conv_id: &str,
        include_system_message: bool,
    ) -> MemoryResult<Vec<StoredMessage>> {
        let mut messages = self.store.get_full_history(conv_id).await?;

        if include_system_message {
            // Get conversation information to check if there's a system message
            let conversation = self.store.get_conversation(conv_id).await?;

            if let Some(system_message) = conversation.system_message {
                // Create system message object, insert at beginning of list
                let system_msg = StoredMessage {
                    id: format!("system-{conv_id}"),
                    conversation_id: conv_id.to_string(),
                    role: MessageRole::System,
                    content: system_message,
                    timestamp: conversation.created_at,
                    sequence: 0, // System message sequence number is 0
                    tokens: None,
                    tool_calls: Vec::new(),
                };

                messages.insert(0, system_msg);
            }
        }

        Ok(messages)
    }

    /// Get complete chat history by user ID
    ///
    /// # Parameters
    /// * `user_id` - User's unique identifier
    /// * `include_system_message` - Whether to include system message at the beginning of returned results
    ///
    /// # Returns
    /// * `MemoryResult<Vec<StoredMessage>>` - Returns complete message list on success, MemoryError on failure
    ///
    /// # Description
    /// Based on the 1:1 relationship between user ID and conversation ID, this method will:
    /// 1. First find the conversation corresponding to the user (regardless of model)
    /// 2. If found, return the complete message history for that conversation
    /// 3. If user has no conversations, return empty message list
    ///
    /// This is a convenience method that avoids the two-step operation of first getting conversation ID then getting history.
    /// Returned messages are sorted by sequence number in ascending order, containing complete conversation history.
    ///
    /// ## System Message Handling
    /// * When `include_system_message` is `true`:
    ///   - If conversation has system message, it will be inserted at the beginning of returned list
    /// * When `include_system_message` is `false`:
    ///   - Only returns user, assistant and tool messages, excluding system messages
    ///
    /// # Errors
    /// * `MemoryError::DatabaseError` - When database query fails
    pub async fn get_user_full_history(
        &self,
        user_id: &str,
        include_system_message: bool,
    ) -> MemoryResult<Vec<StoredMessage>> {
        // 1. Get user's conversation ID
        if let Some(conv) = self
            .store
            .get_recent_conversation_by_user(user_id, None)
            .await?
        {
            // 2. Get complete chat history
            self.get_full_history(&conv.id, include_system_message)
                .await
        } else {
            Ok(Vec::new()) // User has no conversation history
        }
    }

    /// Delete specified conversation and all its related data
    ///
    /// # Parameters
    /// * `conv_id` - Unique identifier of the conversation to delete
    ///
    /// # Returns
    /// * `MemoryResult<()>` - Returns () on success, MemoryError on failure
    ///
    /// # Description
    /// This method will completely delete all data for the conversation, including:
    /// 1. Context data in memory cache
    /// 2. Conversation records and all messages in database
    ///
    /// Note: This operation is irreversible, please use with caution. Due to foreign key constraint cascade deletion,
    /// deleting a conversation will automatically delete all messages under that conversation.
    #[allow(dead_code)]
    pub async fn delete_conversation(&self, conv_id: &str) -> MemoryResult<()> {
        // Remove from cache
        self.context_cache.lock().await.remove(conv_id);

        // Delete from database
        self.store.delete_conversation(conv_id).await
    }

    /// Get all historical messages for full history summarization
    ///
    /// # Parameters
    /// * `conv_id` - Conversation ID
    /// * `current_to_summarize` - Current message list that needs summarization
    ///
    /// # Returns
    /// * `MemoryResult<Vec<StoredMessage>>` - Contains all historical messages that need summarization
    ///
    /// # Description
    /// For full history summarization strategy, we need to get:
    /// 1. All messages in `current_to_summarize`
    /// 2. Historical messages in database with sequence numbers less than minimum sequence number in `current_to_summarize`
    ///
    /// For example:
    /// - If `current_to_summarize` contains messages with sequence numbers [3,4]
    /// - Database contains messages with sequence numbers [1,2,3,4,5,6,...]
    /// - Then return messages with sequence numbers [1,2,3,4] for summarization
    ///
    /// This avoids duplicate inclusion of messages and ensures summary contains complete historical context.
    async fn get_all_historical_messages_for_summary(
        &self,
        conv_id: &str,
        current_to_summarize: &[StoredMessage],
    ) -> MemoryResult<Vec<StoredMessage>> {
        // Get all historical messages from database (excluding system messages, as summaries don't need them)
        let historical_messages = self.store.get_full_history(conv_id).await?;

        // Find minimum sequence number in current_to_summarize
        let min_current_sequence = current_to_summarize
            .iter()
            .map(|msg| msg.sequence)
            .min()
            .unwrap_or(i64::MAX);

        // Only keep historical messages with sequence numbers less than minimum sequence number in current_to_summarize
        let mut filtered_historical: Vec<StoredMessage> = historical_messages
            .into_iter()
            .filter(|msg| msg.sequence < min_current_sequence)
            .collect();

        // Merge filtered historical messages with current messages to summarize
        filtered_historical.extend_from_slice(current_to_summarize);

        // Sort by sequence number to ensure correct message order
        filtered_historical.sort_by_key(|msg| msg.sequence);

        dual_debug!(
            "All messages for summary (filtered):\n{}",
            serde_json::to_string_pretty(&filtered_historical).unwrap()
        );

        Ok(filtered_historical)
    }

    /// Get memory system statistics
    ///
    /// # Returns
    /// * `MemoryResult<MemoryStats>` - Returns statistics on success, MemoryError on failure
    ///
    /// # Description
    /// Returns statistical data for the entire memory system, including:
    /// - Total conversation count and message count
    /// - Token usage statistics
    /// - Database size and other information
    ///
    /// This statistical information can be used for monitoring system usage, capacity planning and performance analysis.
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> MemoryResult<MemoryStats> {
        self.store.get_stats().await
    }
}
