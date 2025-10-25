use serde::{Deserialize, Serialize};
use std::{
    env,
    error::Error,
    fs::{self, File},
    io::Write,
    net::SocketAddrV4,
    path::PathBuf,
};

const DEF_CONFIG_PATH: &str = "~/.config/taskmeister.toml";

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub server_addr: SocketAddrV4,
    pub prompt: String,
    pub history_file: PathBuf,
}

impl Config {
    pub fn load(path: Option<PathBuf>) -> Result<Config, Box<dyn Error>> {
        let config_file = match path {
            Some(p) => p,
            None => expand_home_dir(DEF_CONFIG_PATH),
        };

        if !config_file.is_file() {
            let config = Config {
                server_addr: "127.0.0.1:14242".parse()?,
                prompt: "taskmeister> ".to_string(),
                history_file: expand_home_dir("~/.taskmeister_history"),
            };

            if let Some(parent) = config_file.parent() {
                fs::create_dir_all(parent)?
            }

            File::create(config_file)?.write(toml::to_string(&config)?.as_bytes())?;
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
