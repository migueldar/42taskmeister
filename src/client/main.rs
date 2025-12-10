mod argument_parser;
mod config;
mod connection;

use argument_parser::ParsedArgumets;
use config::Config;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;
use serde::Deserialize;
use rustyline::DefaultEditor;
use rustyline::error::ReadlineError;

use std::error::Error;
use std::fs::File;
use std::process;

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

fn main() -> Result<(), Box<dyn Error>> {
    let parsed_args = match ParsedArgumets::new() {
        Ok(p) => p,
        Err(_) => {
            print_help();
            process::exit(1);
        }
    };

    if parsed_args.help {
        print_help();
        return Ok(());
    }

    let config = Config::load(parsed_args.config_file, parsed_args.server_addr)?;
    let mut connection = Connection::new(config.server_addr)?;
    let mut exit_code = ExitCode::OK;

    if parsed_args.command.is_some() {
        connection.write(&parsed_args.command.unwrap(), &mut exit_code)?;
        process::exit(exit_code.as_i32());
    }

    let mut rl = DefaultEditor::new()?;
    if !config.history_file.exists() {
        File::create(&config.history_file)?;
    }
    rl.load_history(&config.history_file)?;

    loop {
        let readline = rl.readline(&config.prompt);
        match readline {
            Ok(line) => {
                if str_is_spaces(&line) {
                    continue;
                }
                rl.add_history_entry(line.as_str())?;
                if let Err(err) = connection.write(&line, &mut exit_code) {
                    println!("Error: {:?}", err);
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
                println!("Error: {:?}", err);
                exit_code = ExitCode::OTHERERROR;
                break;
            }
        }
    }

    rl.save_history(&config.history_file)?;
    process::exit(exit_code.as_i32());
}
