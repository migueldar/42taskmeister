#![allow(warnings)]

use std::net::{AddrParseError, SocketAddrV4};

pub struct Config {
    pub server_addr: SocketAddrV4,
    pub prompt: String,
    pub history_file: String,
}

impl Config {
    pub fn load(option_config_file: Option<String>, server_addr_arg: Option<String>) -> Result<Config, AddrParseError> {
        //check if file was passed and exists (error if it does)
        let config_file = option_config_file.unwrap_or("~/.config/taskmeister/client.conf".to_string());
        //take default values that are not in file

        Ok(Config {
            server_addr: "127.0.0.1:14242".parse()?,
            prompt: "taskmeister> ".to_string(),
            history_file: "~/.taskmeister_history".to_string(),
        })
    }
}

