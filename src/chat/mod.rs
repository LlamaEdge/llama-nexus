pub mod normal;
pub mod plan;
pub mod planner;
pub mod react;
pub mod shared;
pub mod trace;
mod utils;
pub mod xml_parser;

#[cfg(test)]
mod task_planner_skills_tests;

// Generate a unique chat id for the chat completion request
pub(crate) fn gen_chat_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4())
}
