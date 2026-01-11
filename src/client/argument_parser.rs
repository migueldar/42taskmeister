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
                "-f" => ret.config_file = Some(PathBuf::from(args.next().ok_or(())?)),
                "-c" => ret.command = Some(args.next().ok_or(())?),
                "-h" => {
                    ret.help = true;
                    break;
                }
                server_addr => {
                    ret.server_addr = Some(server_addr.parse().map_err(|err| {
                        eprintln!("Error: {err}\n");
                        ()
                    })?)
                }
            }
        }
        Ok(ret)
    }
}
