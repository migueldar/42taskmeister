use std::{env, net::SocketAddr, path::PathBuf};

#[derive(Debug)]
pub struct ParsedArgumets {
    pub command: Option<String>,
    pub config_file: Option<PathBuf>,
    pub server_addr: Option<SocketAddr>,
    pub help: bool,
}

impl ParsedArgumets {
    fn empty() -> ParsedArgumets {
        ParsedArgumets {
            command: None,
            config_file: None,
            server_addr: None,
            help: false,
        }
    }

    pub fn new() -> Result<ParsedArgumets, ()> {
        let mut ret = ParsedArgumets::empty();
        let mut args = env::args().skip(1);

        while let Some(arg) = args.next() {
            match arg.as_str() {
                "-f" => {
                    ret.config_file = match args.next() {
                        Some(file) => Some(PathBuf::from(file)),
                        None => return Err(()),
                    }
                }
                "-c" => {
                    ret.command = match args.next() {
                        Some(command) => Some(command),
                        None => return Err(()),
                    }
                }
                "-h" => {
                    ret.help = true;
                    break;
                }
                server_addr => {
                    ret.server_addr = match server_addr.parse() {
                        Ok(addr) => Some(addr),
                        Err(err) => {
                            println!("{}", err);
                            return Err(());
                        }
                    }
                }
            }
        }
        Ok(ret)
    }
}
