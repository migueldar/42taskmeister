mod argument_parser;
mod config;
mod connection;

use argument_parser::ParsedArgumets;
use config::Config;
use connection::Connection;
use rustyline::error::ReadlineError;
use rustyline::history::FileHistory;
use rustyline::{DefaultEditor, Editor};

use std::error::Error;
use std::fs::File;
use std::process::{self, ExitCode as PExitCode};

const HELPMESSAGE: &str = r#"usage: cargo run --bin client [OPTIONS...] [server_addr]

OPTIONS
    -f FILE      read config from FILE, if not specified config will be read from ~/.config/taskmeister/client.toml
    -c COMMAND   executes COMMAND 
    -h           displays this message
"#;

#[derive(Copy, Clone)]
pub enum ExitCode {
    OK = 0,
    SIGINT = 1,
    COMMANDERROR = 2,
    OTHERERROR = 3,
}

impl ExitCode {
    fn as_i32(&self) -> i32 {
        return *self as i32;
    }
}

fn str_is_spaces(line: &str) -> bool {
    !line.contains(|c: char| !c.is_whitespace())
}

fn print_help() {
    print!("{}", HELPMESSAGE);
}

fn main() -> PExitCode {
    let parsed_args = match ParsedArgumets::new() {
        Ok(p) => p,
        Err(_) => {
            print_help();
            return PExitCode::FAILURE;
        }
    };

    if parsed_args.help {
        print_help();
        return PExitCode::SUCCESS;
    }

    let config = match Config::load(parsed_args.config_file, parsed_args.server_addr) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Config error: {err}");
            return PExitCode::FAILURE;
        }
    };
    let mut connection = match Connection::new(config.server_addr) {
        Ok(c) => c,
        Err(err) => {
            eprintln!("Connection error: {err}");
            return PExitCode::FAILURE;
        }
    };
    let mut exit_code = ExitCode::OK;

    if parsed_args.command.is_some() {
        connection
            .write(&parsed_args.command.unwrap(), &mut exit_code)
            .map_err(|err| {
                eprintln!("Connection error: {err}");
            })
            .ok();
        process::exit(exit_code.as_i32());
    }

    let readline_init_func = || -> Result<Editor<(), FileHistory>, Box<dyn Error>> {
        let mut ret = DefaultEditor::new()?;
        if !config.history_file.exists() {
            File::create(&config.history_file)?;
        }
        ret.load_history(&config.history_file)?;
        Ok(ret)
    };

    let mut rl = match readline_init_func() {
        Ok(r) => r,
        Err(err) => {
            eprintln!("Readline error: {err}");
            return PExitCode::FAILURE;
        }
    };

    loop {
        let readline = rl.readline(&config.prompt);
        match readline {
            Ok(line) => {
                if str_is_spaces(&line) {
                    continue;
                }
                if let Err(err) = rl.add_history_entry(line.as_str()) {
                    eprintln!("Readline error: {err}");
                    return PExitCode::FAILURE;
                }
                if let Err(err) = connection.write(&line, &mut exit_code) {
                    eprintln!("Connection error: {err}");
                    break;
                }
            }
            Err(ReadlineError::Eof) => {
                break;
            }
            Err(ReadlineError::Interrupted) => {
                exit_code = ExitCode::SIGINT;
                break;
            }
            Err(err) => {
                eprintln!("Readline error: {:?}", err);
                exit_code = ExitCode::OTHERERROR;
                break;
            }
        }
    }

    if let Err(err) = rl.save_history(&config.history_file) {
        eprintln!("Readline error: {err}");
        return PExitCode::FAILURE;
    }
    process::exit(exit_code.as_i32());
}
