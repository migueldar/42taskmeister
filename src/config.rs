#![allow(warnings)]

use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fs::{self, File},
    io::Write,
    net::{AddrParseError, SocketAddrV4},
    path::PathBuf,
};

const def_config_path: &str = "~/.config/taskmeister/client.conf";

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub server_addr: SocketAddrV4,
    pub prompt: String,
    pub history_file: PathBuf,
}

impl Config {
    pub fn load(path: Option<PathBuf>) -> Result<Config, Box<dyn Error>> {
        let config = Config {
            server_addr: "127.0.0.1:14242".parse()?,
            prompt: "taskmeister> ".to_string(),
            history_file: expand_home_dir("~/.taskmeister_history"),
        };

        // check if file was passed and exists (error if it does)
        // take default values that are not in file

        let Some(config_file) = path else {
            let cf = expand_home_dir(def_config_path);

            if let Some(parent) = cf.parent() {
                fs::create_dir_all(parent)?
            }

            File::create(cf)?.write(toml::to_string(&config)?.as_bytes());
            return Ok(config);
        };

        Ok(toml::from_str(fs::read_to_string(config_file)?.as_str())?)
    }
}

// TODO: Dummy implementation, only parsing "-f" flag, extend this if
// needed more flags appart from "-f"
pub fn parse_config_path() -> Option<PathBuf> {
    let mut args = env::args();
    while let Some(s) = args.next() {
        if s == "-f" {
            break;
        }
    }

    args.next().map(|e| PathBuf::from(e))
}

// Only needed for non-shell inputs
fn expand_home_dir(path: &str) -> PathBuf {
    if let Some(s) = path.strip_prefix("~/") {
        if let Some(mut home) = env::home_dir() {
            home.push(PathBuf::from(s));
            return home;
        }
    }
    PathBuf::from(path)
}
