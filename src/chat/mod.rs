pub mod normal;
pub mod react;
mod utils;

pub(crate) use normal::chat;
#[allow(unused_imports)]
pub(crate) use react::chat as react_chat;

// Generate a unique chat id for the chat completion request
pub(crate) fn gen_chat_id() -> String {
    format!("chatcmpl-{}", uuid::Uuid::new_v4())
}
