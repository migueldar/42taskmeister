mod config;

use config::Config;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde::Deserialize;
use std::io::Write;
use std::net::TcpStream;
use std::process;
use taskmeister::{Request, Response, ResponsePart};

#[derive(Copy, Clone)]
enum ExitCodes {
    OK = 0,
    SIGINT = 1,
    COMMANDERROR = 2,
    OTHERERROR = 3,
}

fn line_to_request(line: &str) -> Request {
    let mut splitted_line = line.split_whitespace();
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

fn str_is_spaces(line: &str) -> bool {
    !line.contains(|c: char| !c.is_whitespace())
}

fn process_response(res: &Response, exit_code: &mut ExitCodes) {
    *exit_code = ExitCodes::OK;
    for r in res.iter() {
        println!("{}", r);
        if !matches!(exit_code, ExitCodes::COMMANDERROR) && matches!(r, ResponsePart::Error(_)) {
            *exit_code = ExitCodes::COMMANDERROR;
        }
    }
}

fn main() -> std::io::Result<()> {
    let config: Config = Config::load(None, None).unwrap();

    let mut sock_write: TcpStream = TcpStream::connect(config.server_addr)?;
    let sock_read: TcpStream = sock_write.try_clone()?;
    let mut res = serde_json::Deserializer::from_reader(&sock_read);
    let mut exit_code = ExitCodes::OK;

    let mut rl = DefaultEditor::new().unwrap();
    // rl.load_history(&config.history_file).unwrap();

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
                process_response(&res, &mut exit_code);
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(ReadlineError::Interrupted) => {
                exit_code = ExitCodes::SIGINT;
                break;
            }
            Err(err) => {
                println!("Error: {:?}", err);
                exit_code = ExitCodes::OTHERERROR;
                break;
            }
        }
    }

    // rl.save_history(&config.history_file).unwrap();
    process::exit(exit_code as i32);
}
