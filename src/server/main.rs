#![allow(warnings)]
use std::{io::{Read, Write}, net::{SocketAddrV4, TcpListener, TcpStream}};

use serde::Deserialize;
use taskmeister::{Request, Response, ResponsePart};

fn main() -> std::io::Result<()> {
    let listen_sock_addr: SocketAddrV4 = "127.0.0.1:8888".parse().unwrap();
    let listen_sock: TcpListener = TcpListener::bind(listen_sock_addr)?;
    let mut sock_read: TcpStream = listen_sock.accept()?.0;
    let mut deserializer = serde_json::Deserializer::from_reader(&sock_read);
    let mut sock_write: TcpStream = sock_read.try_clone()?;

    loop {
        let u = Request::deserialize(&mut deserializer)?;
        println!("{:?}", u);
        let res: Response = vec![ResponsePart::Info("hola from server".to_string())];
        sock_write.write(serde_json::to_string(&res)?.as_bytes()).unwrap();
    }
}
