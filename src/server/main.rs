mod config;
mod service;

use config::Config;
use serde::Deserialize;
use std::{
    error::Error,
    io::Write,
    net::{TcpListener, TcpStream},
    thread::{self},
};
use taskmeister::{utils, Request, Response, ResponsePart};

// this is here for client testing purposes
fn process_request(req: &Request) -> Response {
    let mut res: Response = Vec::new();
    for a in req.args.iter() {
        res.push(ResponsePart::Info(a.to_string() + " arg"));
    }
    for a in req.flags.iter() {
        res.push(ResponsePart::Error(a.to_string() + " flag"));
    }
    res
}

fn main() -> Result<(), Box<dyn Error>> {
    let (cfg_path, args) = utils::parse_config_path();
    let mut config = Config::load(cfg_path)?;

    if let Some(a) = args {
        config.server_addr = a.parse()?;
    }

    let services = config.load_services()?;

    println!("{services:#?}");

    let listen_sock: TcpListener = TcpListener::bind(config.server_addr)?;

    loop {
        let sock_read: TcpStream = listen_sock.accept()?.0;
        thread::spawn(move || -> std::io::Result<()> {
            println!("entering thread");
            let mut deserializer = serde_json::Deserializer::from_reader(&sock_read);
            let mut sock_write: TcpStream = sock_read.try_clone()?;

            loop {
                let req = Request::deserialize(&mut deserializer)?;
                println!("Request: {:?}", req);
                let res: Response = process_request(&req);
                sock_write.write(serde_json::to_string(&res)?.as_bytes())?;
            }
        });
    }
}
