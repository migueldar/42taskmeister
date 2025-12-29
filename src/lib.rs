use std::os::fd::AsRawFd;

use serde::{Deserialize, Serialize};

pub mod dir_utils;

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub command: String,
    pub flags: Vec<String>,
    pub args: Vec<String>,
    pub stream: Option<Vec<u8>>,
}

pub type Response = Vec<ResponsePart>;

#[derive(Debug, Serialize, Deserialize)]
pub enum ResponsePart {
    Error(String),
    Info(String),
    Stream(Vec<u8>),
}

impl std::fmt::Display for ResponsePart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResponsePart::Info(message) => write!(f, "{}", message),
            ResponsePart::Error(message) => write!(f, "Error: {}", message),
            ResponsePart::Stream(items) => write!(f, "{}", String::from_utf8_lossy(&items)),
        }
    }
}

trait OkPart {
    fn into_response(self) -> ResponsePart;
}

impl OkPart for () {
    fn into_response(self) -> ResponsePart {
        ResponsePart::Info("OK".to_string())
    }
}

impl OkPart for String {
    fn into_response(self) -> ResponsePart {
        ResponsePart::Info(self)
    }
}

impl OkPart for Vec<u8> {
    fn into_response(self) -> ResponsePart {
        ResponsePart::Stream(self)
    }
}

impl<T, E> From<Result<T, E>> for ResponsePart
where
    T: OkPart,
    E: std::fmt::Display,
{
    fn from(value: Result<T, E>) -> Self {
        match value {
            Ok(ok_part) => ok_part.into_response(),
            Err(err) => ResponsePart::Error(err.to_string()),
        }
    }
}

// Sets fd non blocking returning previous flag set
pub fn set_fd_flag<T>(fd: &T, new_flag: i32) -> i32
where
    T: AsRawFd,
{
    unsafe {
        let fd = fd.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | new_flag);
        flags
    }
}

pub fn clear_fd_flag<T>(fd: &T, new_flag: i32) -> i32
where
    T: AsRawFd,
{
    unsafe {
        let fd = fd.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags & !new_flag);
        flags
    }
}
