pub mod normal;
pub mod plan;
pub mod planner;
pub mod react;
pub mod trace;
mod utils;
pub mod xml_parser;

// Generate a unique chat id for the chat completion request
pub(crate) fn gen_chat_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4())
}
