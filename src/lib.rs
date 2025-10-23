use serde::{Serialize, Deserialize};

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
