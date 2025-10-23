#![allow(warnings)]

use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde::Deserialize;
use std::env;
use std::io::{Read, Write};
use std::net::{SocketAddrV4, TcpStream, ToSocketAddrs};
use taskmeister::{Request, Response};

// const config_file_path: String = "~/.config/taskmeister/client.conf";

struct Config {
    sock_addr: SocketAddrV4,
    prompt: String,
    history_file: String,
}

impl Config {
    fn new() -> Config {
        Config {
            sock_addr: "127.0.0.1:8888".parse().unwrap(),
            prompt: "taskmeister> ".to_string(),
            history_file: "/smt/log".to_string(),
        }
    }
}

fn line_to_request(line: &str) -> Request {
    let mut splitted_line = line.split(" ");
    let mut ret = Request {
        command: splitted_line.next().unwrap().to_string(),
        flags: vec![],
        args: vec![],
    };

    for f in splitted_line {
        if f.starts_with("-") {
            ret.flags.push(f.to_string());
        } else {
            ret.args.push(f.to_string());
        }
    }
    ret
}

fn str_is_spaces(line: &String) -> bool {
    for c in line.chars() {
        if c != ' ' {
            return false;
        }
    }
    return true;
}

fn main() -> std::io::Result<()> {
    let config: Config = Config::new();
    let mut sock_write: TcpStream = TcpStream::connect("127.0.0.1:8888")?;
    let mut rl = DefaultEditor::new().unwrap();
    let mut sock_read: TcpStream = sock_write.try_clone()?;
    let mut res = serde_json::Deserializer::from_reader(&sock_read);

    loop {
        let readline = rl.readline(&config.prompt);
        match readline {
            Ok(line) => {
                if str_is_spaces(&line) {
                    continue;
                }
                rl.add_history_entry(line.as_str()).unwrap();

                let req = line_to_request(&line);
                sock_write
                    .write(serde_json::to_string(&req)?.as_bytes())
                    .unwrap();
                let res: Response = Response::deserialize(&mut res)?;
                println!("{:?}", res);
            }
            Err(ReadlineError::Interrupted) => {
                println!("CTRL-C");
                break;
            }
            Err(ReadlineError::Eof) => {
                println!("CTRL-D");
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                break;
            }
        }
    }
    Ok(())
}
