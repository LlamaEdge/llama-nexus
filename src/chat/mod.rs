pub mod normal;
pub mod react;
mod utils;

// Generate a unique chat id for the chat completion request
pub(crate) fn gen_chat_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4())
}
