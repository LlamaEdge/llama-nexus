use sqlx::{Row, SqlitePool};

use crate::responses::models::{Session, SessionRow};

type Result<T> = std::result::Result<T, sqlx::Error>;

pub struct Database {
    pool: SqlitePool,
}

impl Database {
    pub async fn new(db_path: &str) -> Result<Self> {
        let connection_string = if db_path.starts_with("sqlite:") {
            db_path.to_string()
        } else {
            format!("sqlite:{db_path}?mode=rwc")
        };

        let pool = SqlitePool::connect(&connection_string).await?;

        let db = Database { pool };
        db.create_tables().await?;
        Ok(db)
    }

    async fn create_tables(&self) -> Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS sessions(
                id TEXT PRIMARY KEY,
                session_data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_updated INTEGER NOT NULL
            )",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn save_session(&self, session: &Session) -> Result<()> {
        let session_json =
            serde_json::to_string(session).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;

        let now = chrono::Utc::now().timestamp();

        sqlx::query(
            "INSERT OR REPLACE INTO sessions (id, session_data, created_at, last_updated)
            VALUES (?, ?, ?, ?)",
        )
        .bind(&session.response_id)
        .bind(&session_json)
        .bind(session.created)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let row = sqlx::query("SELECT session_data FROM sessions WHERE id = ?")
            .bind(session_id)
            .fetch_optional(&self.pool)
            .await?;

        if let Some(row) = row {
            let session_data: String = row.get(0);
            let session: Session = serde_json::from_str(&session_data)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
            Ok(Some(session))
        } else {
            Ok(None)
        }
    }

    pub async fn find_session_by_response_id(&self, response_id: &str) -> Result<Option<Session>> {
        let rows = sqlx::query("SELECT session_data FROM sessions")
            .fetch_all(&self.pool)
            .await?;

        for row in rows {
            let session_data: String = row.get(0);
            let session: Session = serde_json::from_str(&session_data)
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

            for message in session.messages.values() {
                if let Some(msg_response_id) = &message.response_id
                    && msg_response_id == response_id
                {
                    return Ok(Some(session));
                }
            }
        }

        Ok(None)
    }

    #[allow(dead_code)]
    pub async fn list_sessions(&self) -> Result<Vec<SessionRow>> {
        let rows = sqlx::query(
            "SELECT id, session_data, created_at, last_updated FROM sessions
            ORDER BY last_updated DESC",
        )
        .fetch_all(&self.pool)
        .await?;

        let mut sessions = Vec::new();
        for row in rows {
            sessions.push(SessionRow {
                id: row.get(0),
                session_data: row.get(1),
                created_at: row.get(2),
                last_updated: row.get(3),
            });
        }

        Ok(sessions)
    }

    #[allow(dead_code)]
    pub async fn delete_session(&self, session_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(session_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::responses::models::Session;

    async fn create_test_database() -> Database {
        Database::new(":memory:")
            .await
            .expect("Failed to create test database")
    }

    fn create_test_session() -> Session {
        let mut session = Session::new(
            "test_session_123".to_string(),
            "test_model".to_string(),
            Some("You are a helpful assistant".to_string()),
        );

        session.add_message(
            "user".to_string(),
            "Hello, world!".to_string(),
            10,
            None,
            None,
        );

        session.add_message(
            "assistant".to_string(),
            "Hello! How can I help you?".to_string(),
            15,
            Some(250),
            Some("resp_456".to_string()),
        );

        session
    }

    #[tokio::test]
    async fn test_save_and_get_session() {
        let db = create_test_database().await;
        let session = create_test_session();
        let session_id = session.response_id.clone();

        let result = db.save_session(&session).await;
        assert!(result.is_ok(), "Saving session should succeed");

        let retrieved = db.get_session(&session_id).await.unwrap();
        assert!(retrieved.is_some(), "Should find the saved session");

        let retrieved_session = retrieved.unwrap();
        assert_eq!(retrieved_session.response_id, session.response_id);
        assert_eq!(retrieved_session.model_used, session.model_used);
        assert_eq!(retrieved_session.messages.len(), session.messages.len());

        let result = db.get_session("nonexistent_id").await.unwrap();
        assert!(
            result.is_none(),
            "Should return None for nonexistent session"
        );
    }

    #[tokio::test]
    async fn test_find_session_by_response_id() {
        let db = create_test_database().await;
        let session = create_test_session();

        db.save_session(&session).await.unwrap();

        let found = db.find_session_by_response_id("resp_456").await.unwrap();
        assert!(
            found.is_some(),
            "Should find session by message response ID"
        );

        let found_session = found.unwrap();
        assert_eq!(found_session.response_id, session.response_id);
    }

    #[tokio::test]
    async fn test_session_serialization_roundtrip() {
        let db = create_test_database().await;
        let original_session = create_test_session();

        db.save_session(&original_session).await.unwrap();
        let retrieved = db
            .get_session(&original_session.response_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(retrieved.response_id, original_session.response_id);
        assert_eq!(retrieved.model_used, original_session.model_used);
        assert_eq!(retrieved.created, original_session.created);
        assert_eq!(retrieved.messages.len(), original_session.messages.len());

        let original_msg = original_session.messages.get("2").unwrap();
        let retrieved_msg = retrieved.messages.get("2").unwrap();
        assert_eq!(retrieved_msg.role, original_msg.role);
        assert_eq!(retrieved_msg.content, original_msg.content);
        assert_eq!(retrieved_msg.tokens, original_msg.tokens);
        assert_eq!(retrieved_msg.response_id, original_msg.response_id);
    }

    #[tokio::test]
    async fn test_concurrent_access() {
        use std::sync::Arc;
        use tokio::task;

        let db = Arc::new(create_test_database().await);
        let mut handles = vec![];

        for i in 0..10 {
            let db_clone = Arc::clone(&db);
            let handle = task::spawn(async move {
                let mut session = Session::new(
                    format!("session_{}", i),
                    "test_model".to_string(),
                    Some("System prompt".to_string()),
                );

                session.add_message(
                    "user".to_string(),
                    format!("Message from thread {}", i),
                    5,
                    None,
                    None,
                );

                session.add_message(
                    "assistant".to_string(),
                    format!("Response from thread {}", i),
                    8,
                    Some(100),
                    Some(format!("resp_{}", i)),
                );

                db_clone.save_session(&session).await.unwrap();

                let retrieved = db_clone.get_session(&session.response_id).await.unwrap();
                assert!(retrieved.is_some());

                let found = db_clone
                    .find_session_by_response_id(&format!("resp_{}", i))
                    .await
                    .unwrap();
                assert!(found.is_some());
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.await.unwrap();
        }

        let sessions = db.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 10);
    }
}
