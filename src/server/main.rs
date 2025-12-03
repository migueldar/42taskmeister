mod config;
mod events;
mod jobs;
mod orchestrate;
mod service;
mod watcher;

use config::Config;
use logger::{LogLevel, Logger};
use orchestrate::{Orchestrator, OrchestratorRequest};
use serde::Deserialize;
use service::Services;
use std::{
    error::Error,
    io::Write,
    net::{TcpListener, TcpStream},
    sync::mpsc,
    thread::{self},
    time::Duration,
};
use taskmeister::{dir_utils, Request, Response, ResponsePart};

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
    logger::info!("cosa {}", 23);

    let (cfg_path, args) = dir_utils::parse_config_path();
    let mut config = Config::load(cfg_path)?;

    if let Some(a) = args {
        config.server_addr = a.parse()?;
    }

    let (orchestrator, requests_tx) = Orchestrator::new(Services::load(config.get_includes())?);

    thread::spawn(move || {
        orchestrator.orchestrate();
    });

    let listen_sock: TcpListener = TcpListener::bind(config.server_addr)?;
    loop {
        let sock_read: TcpStream = listen_sock.accept()?.0;
        let requests_tx = requests_tx.clone();

        let handle = thread::spawn(move || -> std::io::Result<()> {
            println!("entering thread");
            let mut deserializer = serde_json::Deserializer::from_reader(&sock_read);
            let mut sock_write: TcpStream = sock_read.try_clone()?;

            loop {
                let req = Request::deserialize(&mut deserializer)?;
                println!("Request: {:?}", req);
                let res: Response = process_request(&req);
                sock_write.write(serde_json::to_string(&res)?.as_bytes())?;

                // START
                let (cli_tx, cli_rx) = mpsc::channel();
                requests_tx
                    .send(orchestrate::OrchestratorMsg::Request(OrchestratorRequest {
                        action: service::ServiceAction::Start("ls1".to_string()),
                        response_channel: cli_tx,
                    }))
                    .inspect_err(|err| eprintln!("Error sending request to orchestrator! {err:?}"))
                    .ok();

                for resp in cli_rx {
                    println!("Received Orchestrator response:\n{resp:?}");
                    break;
                }

                thread::sleep(Duration::from_secs(1));

                // STOP
                // let (cli_tx, cli_rx) = mpsc::channel();
                // requests_tx
                //     .send(jobs::OrchestratorMsg::Request(OrchestratorRequest {
                //         action: service::ServiceAction::Stop("ls1".to_string()),
                //         response_channel: cli_tx,
                //     }))
                //     .inspect_err(|err| eprintln!("Error sending request to orchestrator! {err:?}"))
                //     .ok();

                // for resp in cli_rx {
                //     println!("Received Orchestrator response:\n{resp:?}");
                //     break;
                // }
            }
        });

        let _ = handle.join().unwrap();
    }
}
