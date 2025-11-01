use serde::{Deserialize, Serialize};

pub mod dir_utils;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub command: String,
    pub flags: Vec<String>,
    pub args: Vec<String>,
}

pub type Response = Vec<ResponsePart>;

#[derive(Debug, Serialize, Deserialize)]
pub enum ResponsePart {
    Error(String),
    Info(String),
}

impl std::fmt::Display for ResponsePart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResponsePart::Info(message) => write!(f, "{}", message),
            ResponsePart::Error(message) => write!(f, "Error: {}", message),
        }
    }
}
