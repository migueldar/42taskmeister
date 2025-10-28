use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(tag = "type", content = "value")]
enum Restart {
    #[default]
    Never,
    Always(u8),
    OnError(u8),
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Service {
    cmd: String,
    clone: u16,
    restart: Restart,
    timeout: u32,
    stop_signal: u32,
    stop_wait: u32,
    // redirect: File,
    variables: Vec<(String, String)>,
    working_dir: PathBuf,
    umask: u16,
}

impl Service {}
