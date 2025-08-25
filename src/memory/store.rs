use std::str::FromStr;

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::{dual_error, memory::types::*};

pub struct MessageStore {
    pool: SqlitePool,
}

impl MessageStore {
    /// 创建一个新的 MessageStore 实例
    ///
    /// # 参数
    /// * `database_path` - SQLite 数据库文件的路径
    ///
    /// # 返回值
    /// * `MemoryResult<Self>` - 成功时返回 MessageStore 实例，失败时返回 MemoryError
    ///
    /// # 说明
    /// 此方法会自动连接到 SQLite 数据库并初始化必要的表结构。
    /// 如果数据库文件不存在，SQLite 会自动创建。
    pub async fn new(database_path: &str) -> MemoryResult<Self> {
        let pool = SqlitePool::connect(&format!("sqlite:{database_path}")).await?;

        let store = Self { pool };
        store.initialize_schema().await?;

        Ok(store)
    }

    async fn initialize_schema(&self) -> MemoryResult<()> {
        // 首先创建基础表结构
        sqlx::query(
            r#"
            CREATE TABLE IF NOT EXISTS conversations (
                id TEXT PRIMARY KEY,
                title TEXT,
                model_name TEXT NOT NULL,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                message_count INTEGER DEFAULT 0,
                total_tokens INTEGER DEFAULT 0,
                summary TEXT,
                last_summary_sequence INTEGER,
                system_message TEXT,
                system_message_hash TEXT,
                system_message_updated_at DATETIME
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
                sequence INTEGER NOT NULL,
                tokens INTEGER,
                tool_calls TEXT,
                FOREIGN KEY (conversation_id) REFERENCES conversations(id) ON DELETE CASCADE
            );
            "#,
        )
        .execute(&self.pool)
        .await?;

        // 添加user_id列（如果不存在）
        let _ = sqlx::query("ALTER TABLE conversations ADD COLUMN user_id TEXT")
            .execute(&self.pool)
            .await;

        // 添加system message相关列（如果不存在）
        let _ = sqlx::query("ALTER TABLE conversations ADD COLUMN system_message TEXT")
            .execute(&self.pool)
            .await;
        let _ = sqlx::query("ALTER TABLE conversations ADD COLUMN system_message_hash TEXT")
            .execute(&self.pool)
            .await;
        let _ =
            sqlx::query("ALTER TABLE conversations ADD COLUMN system_message_updated_at DATETIME")
                .execute(&self.pool)
                .await;

        // 创建索引
        sqlx::query(
            r#"
            CREATE INDEX IF NOT EXISTS idx_messages_conv_seq ON messages(conversation_id, sequence);
            CREATE INDEX IF NOT EXISTS idx_conversations_updated ON conversations(updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_conversations_user_updated ON conversations(user_id, updated_at DESC);
            CREATE INDEX IF NOT EXISTS idx_messages_timestamp ON messages(timestamp DESC);
            "#
        ).execute(&self.pool).await?;

        Ok(())
    }

    /// 创建一个新的对话记录
    ///
    /// # 参数
    /// * `conv` - 要创建的对话信息
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    ///
    /// # 说明
    /// 在数据库中插入一条新的对话记录。对话 ID 必须是唯一的。
    pub async fn create_conversation(&self, conv: &StoredConversation) -> MemoryResult<()> {
        let query = "INSERT INTO conversations (id, user_id, title, model_name, created_at, updated_at, system_message, system_message_hash, system_message_updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";
        sqlx::query(query)
            .bind(&conv.id)
            .bind(&conv.user_id)
            .bind(&conv.title)
            .bind(&conv.model_name)
            .bind(conv.created_at.naive_utc())
            .bind(conv.updated_at.naive_utc())
            .bind(&conv.system_message)
            .bind(&conv.system_message_hash)
            .bind(conv.system_message_updated_at.map(|dt| dt.naive_utc()))
            .execute(&self.pool)
            .await?;

        Ok(())
    }

    /// 存储一条消息到数据库
    ///
    /// # 参数
    /// * `message` - 要存储的消息对象
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    ///
    /// # 说明
    /// 存储消息到数据库并自动更新对应对话的统计信息（消息数量、token 总数等）。
    /// 如果消息包含工具调用，会将其序列化为 JSON 格式存储。
    pub async fn store_message(&self, message: &StoredMessage) -> MemoryResult<()> {
        let tool_calls_json = if message.tool_calls.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&message.tool_calls)?)
        };

        let role = message.role.to_string();
        let tokens = message.tokens.map(|t| t as i64);

        sqlx::query!(
            "INSERT INTO messages (id, conversation_id, role, content, timestamp, sequence, tokens, tool_calls)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            message.id,
            message.conversation_id,
            role,
            message.content,
            message.timestamp,
            message.sequence,
            tokens,
            tool_calls_json
        ).execute(&self.pool).await?;

        // 更新会话统计
        self.update_conversation_stats(&message.conversation_id)
            .await?;

        Ok(())
    }

    /// 根据对话 ID 获取对话的元数据（metadata）
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<StoredConversation>` - 成功时返回对话的元数据对象，失败时返回 MemoryError
    ///
    /// # 说明
    /// 从 conversations 表中查询指定对话的元数据信息，**不包含**具体的聊天消息内容。
    /// 返回的元数据包括对话的基本属性、统计信息和摘要信息。
    /// 这是一个轻量级的单表查询操作，性能开销固定且较小。
    ///
    /// # 错误
    /// * `MemoryError::ConversationNotFound` - 当指定的对话不存在时
    /// * `MemoryError::InvalidData` - 当数据库查询失败时
    pub async fn get_conversation(&self, conv_id: &str) -> MemoryResult<StoredConversation> {
        let row = sqlx::query("SELECT * FROM conversations WHERE id = ?")
            .bind(conv_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| {
                let err_msg = format!("Failed to fetch conversation: {e}");
                dual_error!("{err_msg}");
                MemoryError::InvalidData(err_msg)
            })?;

        match row {
            Some(row) => {
                let id: String = row.try_get("id")?;
                let user_id: Option<String> = row.try_get("user_id").ok();
                let title: Option<String> = row.try_get("title").ok();
                let model_name: String = row.try_get("model_name")?;
                let created_at = row.try_get::<chrono::NaiveDateTime, _>("created_at")?;
                let updated_at = row.try_get::<chrono::NaiveDateTime, _>("updated_at")?;
                let message_count: i64 = row.try_get("message_count")?;
                let total_tokens: i64 = row.try_get("total_tokens")?;
                let summary: Option<String> = row.try_get("summary").ok();
                let last_summary_sequence: Option<i64> = row.try_get("last_summary_sequence").ok();
                let system_message: Option<String> = row.try_get("system_message").ok();
                let system_message_hash: Option<String> = row.try_get("system_message_hash").ok();
                let system_message_updated_at: Option<chrono::NaiveDateTime> =
                    row.try_get("system_message_updated_at").ok();

                Ok(StoredConversation {
                    id,
                    user_id,
                    title,
                    model_name,
                    created_at: DateTime::<Utc>::from_naive_utc_and_offset(created_at, Utc),
                    updated_at: DateTime::<Utc>::from_naive_utc_and_offset(updated_at, Utc),
                    message_count,
                    total_tokens,
                    summary,
                    last_summary_sequence,
                    system_message,
                    system_message_hash,
                    system_message_updated_at: system_message_updated_at
                        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc)),
                })
            }
            None => Err(MemoryError::ConversationNotFound(conv_id.to_string())),
        }
    }

    /// 根据用户ID获取最近对话的元数据（metadata）
    ///
    /// # 参数
    /// * `user_id` - 用户的唯一标识符
    /// * `model_name` - 可选的模型名称过滤条件
    ///
    /// # 返回值
    /// * `MemoryResult<Option<StoredConversation>>` - 成功时返回最近对话的元数据对象，如果没有找到则返回None
    ///
    /// # 说明
    /// 从 conversations 表中查询指定用户最近的对话元数据信息，**不包含**具体的聊天消息内容。
    /// 返回的元数据与 `get_conversation()` 方法相同，包括对话的基本属性、统计信息和摘要信息。
    ///
    /// 查询策略：
    /// - 按 `updated_at` 时间降序排列，返回最新更新的对话
    /// - 支持可选的模型名称过滤
    /// - 这是一个轻量级的单表查询操作，性能开销较小
    ///
    /// 使用场景：
    /// - 恢复用户的活跃对话会话
    /// - 检查用户是否有现有对话
    /// - 获取用户最近对话的概览信息
    ///
    /// # 错误
    /// * `MemoryError::InvalidData` - 当数据库查询失败时
    pub async fn get_recent_conversation_by_user(
        &self,
        user_id: &str,
        model_name: Option<&str>,
    ) -> MemoryResult<Option<StoredConversation>> {
        let (query_str, params): (String, Vec<&str>) = if let Some(model) = model_name {
            ("SELECT * FROM conversations WHERE user_id = ? AND model_name = ? ORDER BY updated_at DESC LIMIT 1".to_string(),
             vec![user_id, model])
        } else {
            (
                "SELECT * FROM conversations WHERE user_id = ? ORDER BY updated_at DESC LIMIT 1"
                    .to_string(),
                vec![user_id],
            )
        };

        let mut query = sqlx::query(&query_str);
        for param in params {
            query = query.bind(param);
        }

        let row = query.fetch_optional(&self.pool).await.map_err(|e| {
            let err_msg = format!("Failed to fetch recent conversation for user {user_id}: {e}");
            dual_error!("{err_msg}");
            MemoryError::InvalidData(err_msg)
        })?;

        match row {
            Some(row) => {
                let id: String = row.try_get("id")?;
                let user_id_db: Option<String> = row.try_get("user_id").ok();
                let title: Option<String> = row.try_get("title").ok();
                let model_name: String = row.try_get("model_name")?;
                let created_at = row.try_get::<chrono::NaiveDateTime, _>("created_at")?;
                let updated_at = row.try_get::<chrono::NaiveDateTime, _>("updated_at")?;
                let message_count: i64 = row.try_get("message_count")?;
                let total_tokens: i64 = row.try_get("total_tokens")?;
                let summary: Option<String> = row.try_get("summary").ok();
                let last_summary_sequence: Option<i64> = row.try_get("last_summary_sequence").ok();
                let system_message: Option<String> = row.try_get("system_message").ok();
                let system_message_hash: Option<String> = row.try_get("system_message_hash").ok();
                let system_message_updated_at: Option<chrono::NaiveDateTime> =
                    row.try_get("system_message_updated_at").ok();

                Ok(Some(StoredConversation {
                    id,
                    user_id: user_id_db,
                    title,
                    model_name,
                    created_at: DateTime::<Utc>::from_naive_utc_and_offset(created_at, Utc),
                    updated_at: DateTime::<Utc>::from_naive_utc_and_offset(updated_at, Utc),
                    message_count,
                    total_tokens,
                    summary,
                    last_summary_sequence,
                    system_message,
                    system_message_hash,
                    system_message_updated_at: system_message_updated_at
                        .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc)),
                }))
            }
            None => Ok(None),
        }
    }

    /// 获取指定对话的完整消息历史
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 成功时返回按序列排序的消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回对话中的所有消息，按照序列号升序排列。
    /// 工具调用信息会从 JSON 格式反序列化为结构化数据。
    pub async fn get_full_history(&self, conv_id: &str) -> MemoryResult<Vec<StoredMessage>> {
        let rows = sqlx::query!(
            "SELECT * FROM messages WHERE conversation_id = ? ORDER BY sequence",
            conv_id
        )
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            let tool_calls: Vec<StoredToolCall> = if let Some(tool_calls_json) = row.tool_calls {
                serde_json::from_str(&tool_calls_json)?
            } else {
                Vec::new()
            };

            let id = row.id.unwrap();
            let timestamp = DateTime::<Utc>::from_naive_utc_and_offset(row.timestamp.unwrap(), Utc);

            messages.push(StoredMessage {
                id,
                conversation_id: row.conversation_id,
                role: MessageRole::from_str(&row.role).map_err(|e| {
                    let err_msg = format!("Failed to parse message role: {e}");
                    dual_error!("{err_msg}");
                    MemoryError::InvalidData(err_msg)
                })?,
                content: row.content,
                timestamp,
                sequence: row.sequence,
                tokens: row.tokens.map(|t| t as usize),
                tool_calls,
            });
        }

        Ok(messages)
    }

    /// 获取指定对话的最近N条消息
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    /// * `limit` - 最大返回消息数量
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 成功时返回按序列排序的消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回对话中最近的N条消息，按照序列号升序排列。
    /// 工具调用信息会从 JSON 格式反序列化为结构化数据。
    pub async fn get_recent_messages(
        &self,
        conv_id: &str,
        limit: usize,
    ) -> MemoryResult<Vec<StoredMessage>> {
        let limit_i64 = limit as i64;
        let rows = sqlx::query(
            "SELECT * FROM messages WHERE conversation_id = ? ORDER BY sequence DESC LIMIT ?",
        )
        .bind(conv_id)
        .bind(limit_i64)
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows.into_iter().rev() {
            // 反转以保持时间顺序
            let tool_calls_json: Option<String> = row.try_get("tool_calls").ok();
            let tool_calls: Vec<StoredToolCall> = if let Some(json_str) = tool_calls_json
                && !json_str.is_empty()
            {
                serde_json::from_str(&json_str)?
            } else {
                Vec::new()
            };

            let id: String = row.try_get("id")?;
            let conversation_id: String = row.try_get("conversation_id")?;
            let role_str: String = row.try_get("role")?;
            let content: String = row.try_get("content")?;
            let timestamp = row.try_get::<chrono::NaiveDateTime, _>("timestamp")?;
            let sequence: i64 = row.try_get("sequence")?;
            let tokens: Option<i64> = row.try_get("tokens").ok();

            messages.push(StoredMessage {
                id,
                conversation_id,
                role: MessageRole::from_str(&role_str).map_err(|e| {
                    let err_msg = format!("Failed to parse message role: {e}");
                    dual_error!("{err_msg}");
                    MemoryError::InvalidData(err_msg)
                })?,
                content,
                timestamp: DateTime::<Utc>::from_naive_utc_and_offset(timestamp, Utc),
                sequence,
                tokens: tokens.map(|t| t as usize),
                tool_calls,
            });
        }

        Ok(messages)
    }

    /// 获取从指定序列号开始的消息
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    /// * `from_sequence` - 起始序列号（包含）
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<StoredMessage>>` - 成功时返回消息列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 用于获取对话中从某个特定序列号开始的所有消息，常用于增量加载或断点续传场景。
    /// 返回的消息按序列号升序排列。
    #[allow(dead_code)]
    pub async fn get_messages_from_sequence(
        &self,
        conv_id: &str,
        from_sequence: i64,
    ) -> MemoryResult<Vec<StoredMessage>> {
        let rows = sqlx::query!(
            "SELECT * FROM messages WHERE conversation_id = ? AND sequence >= ? ORDER BY sequence",
            conv_id,
            from_sequence
        )
        .fetch_all(&self.pool)
        .await?;

        let mut messages = Vec::new();
        for row in rows {
            let tool_calls: Vec<StoredToolCall> = if let Some(tool_calls_json) = &row.tool_calls {
                serde_json::from_str(tool_calls_json)?
            } else {
                Vec::new()
            };

            let id = row.id.unwrap();
            let role = MessageRole::from_str(&row.role).map_err(|e| {
                let err_msg = format!("Failed to parse message role: {e}");
                dual_error!("{err_msg}");
                MemoryError::InvalidData(err_msg)
            })?;
            let timestamp = DateTime::<Utc>::from_naive_utc_and_offset(row.timestamp.unwrap(), Utc);
            let tokens = row.tokens.map(|t| t as usize);

            messages.push(StoredMessage {
                id,
                conversation_id: row.conversation_id,
                role,
                content: row.content,
                timestamp,
                sequence: row.sequence,
                tokens,
                tool_calls,
            });
        }

        Ok(messages)
    }

    /// 获取指定对话的下一个可用序列号
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<i64>` - 成功时返回下一个序列号，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回对话中下一个可用的序列号，用于确保新消息的序列号唯一且连续。
    /// 如果对话中没有消息，返回 1。
    pub async fn get_next_sequence(&self, conv_id: &str) -> MemoryResult<i64> {
        let row = sqlx::query!(
            "SELECT COALESCE(MAX(sequence), 0) + 1 as next_seq FROM messages WHERE conversation_id = ?",
            conv_id
        ).fetch_one(&self.pool).await?;

        Ok(row.next_seq)
    }

    /// 更新对话的摘要信息
    ///
    /// # 参数
    /// * `conv_id` - 对话的唯一标识符
    /// * `summary` - 新的摘要内容
    /// * `last_sequence` - 摘要所覆盖的最后一个消息序列号
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    ///
    /// # 说明
    /// 更新对话的摘要信息，通常在消息过多需要压缩历史时调用。
    /// 同时会更新对话的最后修改时间。
    pub async fn update_conversation_summary(
        &self,
        conv_id: &str,
        summary: &str,
        last_sequence: Option<i64>,
    ) -> MemoryResult<()> {
        sqlx::query!(
            "UPDATE conversations SET summary = ?, last_summary_sequence = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            summary,
            last_sequence,
            conv_id
        ).execute(&self.pool).await?;

        Ok(())
    }

    /// 更新对话的系统消息
    ///
    /// # 参数
    /// * `conv_id` - 目标对话的 ID
    /// * `system_message` - 新的系统消息内容，如果为 None 表示清除系统消息
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    ///
    /// # 说明
    /// 更新对话的系统消息并计算内容哈希以便检测变化。
    /// 如果系统消息内容发生变化，会更新哈希值和更新时间。
    pub async fn update_system_message(
        &self,
        conv_id: &str,
        system_message: Option<&str>,
    ) -> MemoryResult<()> {
        let (system_msg, system_hash) = if let Some(msg) = system_message {
            let hash = format!("{:x}", md5::compute(msg.as_bytes()));
            (Some(msg), Some(hash))
        } else {
            (None, None)
        };

        sqlx::query(
            "UPDATE conversations SET system_message = ?, system_message_hash = ?, system_message_updated_at = CURRENT_TIMESTAMP, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
        )
        .bind(system_msg)
        .bind(system_hash)
        .bind(conv_id)
        .execute(&self.pool).await?;

        Ok(())
    }

    /// 获取对话列表摘要
    ///
    /// # 参数
    /// * `limit` - 返回的最大对话数量，None 表示使用默认值 100
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<ConversationSummary>>` - 成功时返回对话摘要列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回按最后更新时间降序排列的对话摘要列表，用于在 UI 中显示对话列表。
    /// 每个摘要包含对话的基本信息但不包含详细的消息内容。
    #[allow(dead_code)]
    pub async fn list_conversations(
        &self,
        limit: Option<usize>,
    ) -> MemoryResult<Vec<ConversationSummary>> {
        let limit = limit.unwrap_or(100) as i64;

        let rows = sqlx::query!(
            "SELECT id, user_id, title, model_name, message_count, updated_at, created_at
             FROM conversations
             ORDER BY updated_at DESC
             LIMIT ?",
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        let mut summaries = Vec::new();
        for row in rows {
            let id = row.id.unwrap();
            let user_id: Option<String> = row.user_id;
            let message_count = row.message_count.unwrap();
            let created_at =
                DateTime::<Utc>::from_naive_utc_and_offset(row.created_at.unwrap(), Utc);
            let updated_at =
                DateTime::<Utc>::from_naive_utc_and_offset(row.updated_at.unwrap(), Utc);

            summaries.push(ConversationSummary {
                id,
                user_id,
                title: row.title,
                model_name: row.model_name,
                message_count,
                last_message_at: updated_at,
                created_at,
            });
        }

        Ok(summaries)
    }

    /// 列出指定用户的所有对话摘要
    ///
    /// # 参数
    /// * `user_id` - 用户的唯一标识符
    /// * `limit` - 返回的最大对话数量，None 表示使用默认限制
    ///
    /// # 返回值
    /// * `MemoryResult<Vec<ConversationSummary>>` - 成功时返回该用户的对话摘要列表，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回指定用户按最后更新时间降序排列的对话摘要列表。
    /// 每个摘要包含对话的基本信息但不包含详细的消息内容。
    pub async fn list_conversations_by_user(
        &self,
        user_id: &str,
        limit: Option<usize>,
    ) -> MemoryResult<Vec<ConversationSummary>> {
        let limit = limit.unwrap_or(100) as i64;

        let rows = sqlx::query!(
            "SELECT id, user_id, title, model_name, message_count, updated_at, created_at
             FROM conversations
             WHERE user_id = ?
             ORDER BY updated_at DESC
             LIMIT ?",
            user_id,
            limit
        )
        .fetch_all(&self.pool)
        .await?;

        let mut summaries = Vec::new();
        for row in rows {
            let id = row.id.unwrap();
            let user_id_db: Option<String> = row.user_id;
            let message_count = row.message_count.unwrap();
            let created_at =
                DateTime::<Utc>::from_naive_utc_and_offset(row.created_at.unwrap(), Utc);
            let updated_at =
                DateTime::<Utc>::from_naive_utc_and_offset(row.updated_at.unwrap(), Utc);

            summaries.push(ConversationSummary {
                id,
                user_id: user_id_db,
                title: row.title,
                model_name: row.model_name,
                message_count,
                last_message_at: updated_at,
                created_at,
            });
        }

        Ok(summaries)
    }

    /// 获取内存存储系统的统计信息
    ///
    /// # 返回值
    /// * `MemoryResult<MemoryStats>` - 成功时返回统计信息，失败时返回 MemoryError
    ///
    /// # 说明
    /// 返回包含对话总数、消息总数、token 总数等统计信息的结构。
    /// 某些统计项（如工具调用数量、数据库大小）当前为简化实现，可能返回默认值。
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> MemoryResult<MemoryStats> {
        let conv_count = sqlx::query!("SELECT COUNT(*) as count FROM conversations")
            .fetch_one(&self.pool)
            .await?
            .count;

        let msg_count = sqlx::query!("SELECT COUNT(*) as count FROM messages")
            .fetch_one(&self.pool)
            .await?
            .count;

        let total_tokens = sqlx::query!("SELECT SUM(total_tokens) as total FROM conversations")
            .fetch_one(&self.pool)
            .await?
            .total
            .unwrap_or(0);

        // 简化版统计，实际实现可以更详细
        Ok(MemoryStats {
            total_conversations: conv_count,
            total_messages: msg_count,
            total_tool_calls: 0, // 需要解析JSON统计
            total_tokens,
            database_size_mb: 0.0, // 需要文件系统查询
            most_used_tools: vec![],
            conversations_by_model: vec![],
        })
    }

    async fn update_conversation_stats(&self, conv_id: &str) -> MemoryResult<()> {
        let row = sqlx::query(
            "SELECT COUNT(*) as msg_count, SUM(COALESCE(tokens, 0)) as total_tokens
             FROM messages WHERE conversation_id = ?",
        )
        .bind(conv_id)
        .fetch_one(&self.pool)
        .await?;

        let msg_count: i64 = row.get(0);
        let total_tokens: Option<i64> = row.get(1);
        let total_tokens: i64 = total_tokens.unwrap_or(0);

        sqlx::query!(
            "UPDATE conversations SET message_count = ?, total_tokens = ?, updated_at = CURRENT_TIMESTAMP WHERE id = ?",
            msg_count,
            total_tokens,
            conv_id
        ).execute(&self.pool).await?;

        Ok(())
    }

    /// 删除指定的对话及其所有消息
    ///
    /// # 参数
    /// * `conv_id` - 要删除的对话的唯一标识符
    ///
    /// # 返回值
    /// * `MemoryResult<()>` - 成功时返回 ()，失败时返回 MemoryError
    ///
    /// # 说明
    /// 由于设置了外键约束的级联删除，删除对话记录时会自动删除该对话下的所有消息。
    /// 此操作不可逆，请谨慎使用。
    #[allow(dead_code)]
    pub async fn delete_conversation(&self, conv_id: &str) -> MemoryResult<()> {
        sqlx::query!("DELETE FROM conversations WHERE id = ?", conv_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
