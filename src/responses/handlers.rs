use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::Json};
use endpoints::chat::{
    ChatCompletionRequest, ChatCompletionRequestMessage, ChatCompletionUserMessageContent,
};

use crate::{
    AppState as MainAppState,
    responses::{
        db::Database,
        models::{ResponseReply, ResponseRequest, Session},
    },
    server::RoutingPolicy,
};

pub struct AppState {
    pub db: Database,
    pub main_state: Arc<MainAppState>,
}

pub async fn responses_handler(
    State(state): State<Arc<AppState>>,
    Json(req): Json<ResponseRequest>,
) -> Result<Json<ResponseReply>, (StatusCode, String)> {
    let model = req.model.clone();

    let response_id = format!("resp_{}", uuid::Uuid::new_v4().simple());

    let mut session = if let Some(prev_id) = &req.previous_response_id {
        match state.db.find_session_by_response_id(prev_id).await {
            Ok(Some(existing_session)) => existing_session,
            Ok(None) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    format!("Previous response ID not found: {prev_id}"),
                ));
            }
            Err(e) => {
                return Err((
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Database error: {e}"),
                ));
            }
        }
    } else {
        Session::new(response_id.clone(), model.clone(), req.instructions)
    };

    let user_tokens = estimate_tokens(&req.input);
    session.add_message(
        "user".to_string(),
        req.input.clone(),
        user_tokens,
        None,
        None,
    );

    let conversation = session.get_conversation_history();

    let mut messages = Vec::new();
    for (role, content) in conversation {
        match role.as_str() {
            "system" => {
                let system_msg = ChatCompletionRequestMessage::new_system_message(content, None);
                messages.push(system_msg);
            }
            "user" => {
                let user_msg = ChatCompletionRequestMessage::new_user_message(
                    ChatCompletionUserMessageContent::Text(content),
                    None,
                );
                messages.push(user_msg);
            }
            "assistant" => {
                let assistant_msg =
                    ChatCompletionRequestMessage::new_assistant_message(Some(content), None, None);
                messages.push(assistant_msg);
            }
            _ => {}
        }
    }

    let chat_request = ChatCompletionRequest {
        model: Some(model.clone()),
        messages,
        user: Some("responses-api".to_string()),
        stream: Some(false),
        ..Default::default()
    };

    let chat_result = match call_chat_backend(&state.main_state, chat_request).await {
        Ok(result) => result,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Chat backend error: {e}"),
            ));
        }
    };

    let output_tokens = estimate_tokens(&chat_result);
    session.add_message(
        "assistant".to_string(),
        chat_result.clone(),
        output_tokens,
        None,
        Some(response_id.clone()),
    );

    let final_result = chat_result;

    if let Err(e) = state.db.save_session(&session).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Failed to save session: {e}"),
        ));
    }

    let response = ResponseReply::new(
        response_id,
        model,
        final_result,
        user_tokens,
        output_tokens,
        req.previous_response_id,
    );

    Ok(Json(response))
}

fn estimate_tokens(text: &str) -> i32 {
    (text.len() as f32 / 4.0).ceil() as i32
}

async fn call_chat_backend(
    main_state: &Arc<MainAppState>,
    request: ChatCompletionRequest,
) -> Result<String, String> {
    let servers = main_state.server_group.read().await;
    let chat_servers = match servers.get(&crate::server::ServerKind::chat) {
        Some(servers) => servers,
        None => return Err("No chat server available".to_string()),
    };

    let target_server = match chat_servers.next().await {
        Ok(server) => server,
        Err(e) => return Err(format!("Failed to get chat server: {e}")),
    };

    let url = format!(
        "{}/chat/completions",
        target_server.url.trim_end_matches('/')
    );

    let client = reqwest::Client::new();
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&request)
        .send()
        .await
        .map_err(|e| format!("Request failed: {e}"))?;

    if !response.status().is_success() {
        let error_text = response
            .text()
            .await
            .unwrap_or_else(|_| "Unknown error".to_string());
        return Err(format!("Chat API Error: {error_text}"));
    }

    let chat_response: endpoints::chat::ChatCompletionObject = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    let text = chat_response
        .choices
        .first()
        .and_then(|choice| choice.message.content.as_ref())
        .map(|content| content.to_string())
        .unwrap_or_else(|| "No response content".to_string());

    Ok(text)
}

pub async fn health_handler() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "service": "responses-api"
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens() {
        assert_eq!(estimate_tokens(""), 0);

        assert_eq!(estimate_tokens("a"), 1);

        assert_eq!(estimate_tokens("test"), 1);
        assert_eq!(estimate_tokens("hello"), 2);

        assert_eq!(estimate_tokens("This is a test message"), 6);

        assert_eq!(estimate_tokens("Hello, world!"), 4);
    }

    #[test]
    fn test_health_handler() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let response = runtime.block_on(health_handler());

        let json_value = response.0;
        assert_eq!(json_value["status"], "ok");
        assert_eq!(json_value["service"], "responses-api");
    }
}
