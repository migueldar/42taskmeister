use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::{self, File},
    io::Write,
    net::SocketAddr,
    path::{Path, PathBuf},
};
use taskmeister::dir_utils;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub server_addr: SocketAddr,
    pub prompt: String,
    pub history_file: PathBuf,
}

pub const DEFAULT_CONFIG_PATH: &str = "~/.config/taskmeister/client.toml";
pub const DEFAULT_SERVER_ADDR: &str = "127.0.0.1:14242";
pub const DEFAULT_PROMPT: &str = "taskmeister> ";
pub const DEFAULT_HISTORY_FILE: &str = "~/.taskmeister_history";

impl Config {
    pub fn load(
        path: Option<PathBuf>,
        server_addr: Option<SocketAddr>,
    ) -> Result<Config, Box<dyn Error>> {
        let mut config: Config;
        let is_path = path.is_some();
        let config_file = path.unwrap_or(PathBuf::from(dir_utils::expand_home_dir(Path::new(
            DEFAULT_CONFIG_PATH,
        ))));

        if !config_file.is_file() {
            // Early return if the path was inserted with the flag and does not exist
            if is_path {
                return Err(format!("File {:?} does not exist!", config_file).into());
            }

            // If path is the default one and does not exist create it with default values
            if let Some(parent) = config_file.parent() {
                fs::create_dir_all(parent)?
            }

            config = Config {
                server_addr: DEFAULT_SERVER_ADDR.parse()?,
                prompt: DEFAULT_PROMPT.parse()?,
                history_file: dir_utils::expand_home_dir(Path::new(DEFAULT_HISTORY_FILE)),
            };

            File::create(&config_file)?.write(toml::to_string(&config)?.as_bytes())?;
            println!("Created default configuration file in: {config_file:?}");
        } else {
            config = toml::from_str(&fs::read_to_string(config_file)?)?;
        }

        if let Some(override_server_addr) = server_addr {
            config.server_addr = override_server_addr;
        }
        return Ok(config);
    }
}
