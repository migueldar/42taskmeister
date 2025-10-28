use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::{self, File},
    io::Write,
    net::SocketAddrV4,
    path::{Path, PathBuf},
};
use taskmeister::utils;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub server_addr: SocketAddrV4,
    pub prompt: String,
    pub history_file: PathBuf,
}

// Default values
pub const CONFIG_PATH: &str = "~/.config/taskmeiker/client.toml";
pub const SERVER_ADDR: &str = "127.0.0.1:14242";
pub const PROMPT: &str = "taskmeister>";
pub const HISTORY_FILE: &str = "~/.taskmeister_history";

impl Config {
    pub fn load(path: Option<PathBuf>) -> Result<Config, Box<dyn Error>> {
        let is_flag = path.is_some();

        // Does path contain something?
        let config_file = path.unwrap_or(PathBuf::from(utils::expand_home_dir(Path::new(
            CONFIG_PATH,
        ))));

        if !config_file.is_file() {
            // Early return if the path was inserted with the flag and does not exist
            if is_flag {
                return Err(format!("File {:?} does not exist!", config_file).into());
            }

            // If path is the default one and does not exist create it with default values
            if let Some(parent) = config_file.parent() {
                fs::create_dir_all(parent)?
            }

            let c = Config {
                server_addr: SERVER_ADDR.parse()?,
                prompt: PROMPT.parse()?,
                history_file: utils::expand_home_dir(Path::new(HISTORY_FILE)),
            };

            File::create(&config_file)?.write(toml::to_string(&c)?.as_bytes())?;
            println!("Created default configuration file in: {config_file:?}");
            return Ok(c);
        }

        Ok(toml::from_str(&fs::read_to_string(config_file)?)?)
    }
}
