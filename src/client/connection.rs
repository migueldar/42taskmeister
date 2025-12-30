use std::{
    error::Error,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    os::fd::AsRawFd,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
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
        stream: None,
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

/// Return true when the response is a streaming message and false for any
/// other message
fn process_response(res: &Response, exit_code: &mut ExitCode) -> bool {
    // Server should not, but empty messages can lead to bugs when streaming
    if res.len() == 0 {
        return true;
    }

    *exit_code = ExitCode::OK;
    let mut last_message: &ResponsePart = &ResponsePart::Info("OK".to_string());

    for r in res {
        println!("{}", r);

        last_message = r;

        if !matches!(exit_code, ExitCode::COMMANDERROR) && matches!(r, ResponsePart::Error(_)) {
            *exit_code = ExitCode::COMMANDERROR;
        }
    }

    if matches!(last_message, ResponsePart::Stream(_)) {
        true
    } else {
        false
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
        let mut handle = None;
        let stop_stdio = Arc::new(AtomicBool::new(false));

        if req.command == "at" || req.command == "attach" {
            let mut sock = self.sock_write.try_clone()?;
            let stop_stdio_thread = stop_stdio.clone();

            handle = Some(thread::spawn(move || {
                let mut buff = [0; 1024];
                let mut stdin = io::stdin().lock();
                taskmeister::set_fd_flag(&stdin.as_raw_fd(), libc::O_NONBLOCK);

                while !stop_stdio_thread.load(Ordering::Relaxed) {
                    match stdin.read(&mut buff) {
                        Ok(0) => break,
                        Ok(bytes) => {
                            sock.write(
                                serde_json::to_string(&Request {
                                    command: String::new(),
                                    flags: Vec::new(),
                                    args: req.args.clone(), // The alias
                                    stream: Some(buff[..bytes].to_vec()),
                                })
                                .inspect_err(|err| eprintln!("Error: Stdin forward: {err}"))
                                .unwrap_or_default()
                                .as_bytes(),
                            )
                            .inspect_err(|err| eprintln!("Error: Stdin forward: {err}"))
                            .ok();
                        }
                        Err(err) if err.kind() == io::ErrorKind::WouldBlock => (),
                        Err(err) => {
                            eprintln!("Error: Stdin forward: {err}");
                            break;
                        }
                    };

                    thread::sleep(Duration::from_millis(100));
                }

                taskmeister::clear_fd_flag(&stdin.as_raw_fd(), libc::O_NONBLOCK);
            }));
        }

        let mut streaming = true;
        while streaming {
            let res = Response::deserialize(&mut self.deserializer)?;
            streaming = process_response(&res, exit_code);
        }

        stop_stdio.store(true, Ordering::Relaxed);

        if let Some(handle) = handle {
            let _ = handle.join();
        }

        Ok(())
    }
}
