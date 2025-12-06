use logger::LogLevel;
use serde::{Deserialize, Serialize};
use std::{
    error::Error,
    fs::{self, File},
    io::Write,
    net::SocketAddrV4,
    path::{Path, PathBuf},
};
use taskmeister::dir_utils;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    #[serde(skip)]
    config_path: PathBuf,
    pub server_addr: SocketAddrV4,
    pub logs: Option<PathBuf>,
    pub syslog: bool,
    pub log_level: LogLevel,
    include: Include,
    start: Start,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Include {
    pub paths: Vec<PathBuf>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Start {
    pub services: Vec<String>,
}

// Default values
pub const CONFIG_PATH: &str = "~/.config/taskmeiker/server.toml";
pub const SERVER_ADDR: &str = "127.0.0.1:14242";

impl Config {
    pub fn load(path: Option<PathBuf>) -> Result<Config, Box<dyn Error>> {
        let is_flag = path.is_some();

        // Does path contain something?
        let config_file = path.unwrap_or(PathBuf::from(dir_utils::expand_home_dir(Path::new(
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
                config_path: config_file,
                server_addr: SERVER_ADDR.parse()?,
                logs: None,
                syslog: false,
                log_level: LogLevel::Info,
                include: Include { paths: Vec::new() },
                start: Start {
                    services: Vec::new(),
                },
            };

            File::create(&c.config_path)?.write(toml::to_string(&c)?.as_bytes())?;
            println!("Created default configuration file in: {:?}", c.config_path);
            return Ok(c);
        }

        Ok(Config {
            config_path: config_file.clone(),
            ..toml::from_str(&fs::read_to_string(&config_file)?)?
        })
    }
    pub fn get_includes(&self) -> &Vec<PathBuf> {
        &self.include.paths
    }
}
