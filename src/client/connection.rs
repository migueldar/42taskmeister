use std::{
    error::Error,
    io::Write,
    net::{SocketAddr, TcpStream},
};

use serde::Deserialize;
use serde_json::{Deserializer, de::IoRead};
use taskmeister::{Request, Response, ResponsePart};

use crate::ExitCode;

pub struct Connection {
    sock_write: TcpStream,
    deserializer: Deserializer<IoRead<TcpStream>>,
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

fn process_response(res: &Response, exit_code: &mut ExitCode) {
    *exit_code = ExitCode::OK;
    for r in res {
        println!("{}", r);
        if !matches!(exit_code, ExitCode::COMMANDERROR) && matches!(r, ResponsePart::Error(_)) {
            *exit_code = ExitCode::COMMANDERROR;
        }
    }
}

impl Connection {
    pub fn new(server_addr: SocketAddr) -> Result<Connection, Box<dyn Error>> {
        let sock_write: TcpStream = TcpStream::connect(server_addr)?;
        let sock_read: TcpStream = sock_write.try_clone()?;
        let deserializer = serde_json::Deserializer::from_reader(sock_read);
        Ok(Connection {
            sock_write,
            deserializer,
        })
    }

    pub fn write(&mut self, line: &str, exit_code: &mut ExitCode) -> Result<(), Box<dyn Error>> {
        let req = line_to_request(line);
        self.sock_write
            .write(serde_json::to_string(&req)?.as_bytes())?;
        let res = Response::deserialize(&mut self.deserializer)?;
        process_response(&res, exit_code);
        Ok(())
    }
}
