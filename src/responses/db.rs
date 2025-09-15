use std::sync::Mutex;

use rusqlite::{Connection, Result, params};

use crate::responses::models::{Session, SessionRow};

pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    pub fn new(db_path: &str) -> Result<Self> {
        let conn = Connection::open(db_path)?;
        let db = Database {
            conn: Mutex::new(conn),
        };
        db.create_tables()?;
        Ok(db)
    }

    pub fn create_tables(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "CREATE TABLE IF NOT EXISTS sessions(
                id TEXT PRIMARY KEY,
                session_data TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                last_updated INTEGER NOT NULL
            )",
            [],
        )?;
        Ok(())
    }

    pub fn save_session(&self, session: &Session) -> Result<()> {
        let session_json = serde_json::to_string(session)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let now = chrono::Utc::now().timestamp();
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT OR REPLACE INTO sessions (id, session_data, created_at, last_updated)
            VALUES (?1, ?2, ?3, ?4)",
            params![session.response_id, session_json, session.created, now],
        )?;
        Ok(())
    }

    #[allow(dead_code)] 
    pub fn get_session(&self, session_id: &str) -> Result<Option<Session>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT session_data FROM sessions WHERE id = ?1")?;

        let mut session_iter = stmt.query_map([session_id], |row| {
            let session_data: String = row.get(0)?;
            Ok(session_data)
        })?;

        if let Some(session_result) = session_iter.next() {
            let session_data = session_result?;
            let session: Session = serde_json::from_str(&session_data).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            return Ok(Some(session));
        }

        Ok(None)
    }

    pub fn find_session_by_response_id(&self, response_id: &str) -> Result<Option<Session>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT session_data FROM sessions")?;

        let session_iter = stmt.query_map([], |row| {
            let session_data: String = row.get(0)?;
            Ok(session_data)
        })?;

        for session_result in session_iter {
            let session_data = session_result?;
            let session: Session = serde_json::from_str(&session_data).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;

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
    pub fn list_sessions(&self) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_data, created_at, last_updated FROM sessions
            ORDER BY last_updated DESC",
        )?;

        let session_iter = stmt.query_map([], |row| {
            Ok(SessionRow {
                id: row.get(0)?,
                session_data: row.get(1)?,
                created_at: row.get(2)?,
                last_updated: row.get(3)?,
            })
        })?;

        let mut sessions = Vec::new();
        for session in session_iter {
            sessions.push(session?);
        }

        Ok(sessions)
    }

    #[allow(dead_code)] 
    pub fn delete_session(&self, session_id: &str) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::responses::models::Session;

    fn create_test_database() -> Database {
        Database::new(":memory:").expect("Failed to create test database")
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

    #[test]
    fn test_save_and_get_session() {
        let db = create_test_database();
        let session = create_test_session();
        let session_id = session.response_id.clone();

        let result = db.save_session(&session);
        assert!(result.is_ok(), "Saving session should succeed");

        let retrieved = db.get_session(&session_id).unwrap();
        assert!(retrieved.is_some(), "Should find the saved session");

        let retrieved_session = retrieved.unwrap();
        assert_eq!(retrieved_session.response_id, session.response_id);
        assert_eq!(retrieved_session.model_used, session.model_used);
        assert_eq!(retrieved_session.messages.len(), session.messages.len());

        let result = db.get_session("nonexistent_id").unwrap();
        assert!(
            result.is_none(),
            "Should return None for nonexistent session"
        );
    }

    #[test]
    fn test_find_session_by_response_id() {
        let db = create_test_database();
        let session = create_test_session();

        db.save_session(&session).unwrap();

        let found = db.find_session_by_response_id("resp_456").unwrap();
        assert!(
            found.is_some(),
            "Should find session by message response ID"
        );

        let found_session = found.unwrap();
        assert_eq!(found_session.response_id, session.response_id);
    }

    #[test]
    fn test_session_serialization_roundtrip() {
        let db = create_test_database();
        let original_session = create_test_session();

        db.save_session(&original_session).unwrap();
        let retrieved = db
            .get_session(&original_session.response_id)
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

    #[test]
    fn test_concurrent_access() {
        use std::{sync::Arc, thread};

        let db = Arc::new(create_test_database());
        let mut handles = vec![];

        for i in 0..10 {
            let db_clone = Arc::clone(&db);
            let handle = thread::spawn(move || {
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

                db_clone.save_session(&session).unwrap();

                let retrieved = db_clone.get_session(&session.response_id).unwrap();
                assert!(retrieved.is_some());

                let found = db_clone
                    .find_session_by_response_id(&format!("resp_{}", i))
                    .unwrap();
                assert!(found.is_some());
            });
            handles.push(handle);
        }

        for handle in handles {
            handle.join().unwrap();
        }

        let sessions = db.list_sessions().unwrap();
        assert_eq!(sessions.len(), 10);
    }
}
