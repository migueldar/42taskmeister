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
use service::{ServiceAction, Services};
use std::{
    error::Error,
    io::Write,
    net::{TcpListener, TcpStream},
    sync::{Arc, mpsc},
    thread::{self},
    time::Duration,
};
use taskmeister::{Request, Response, ResponsePart, dir_utils};

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
    let (cfg_path, args) = dir_utils::parse_config_path();
    let mut config = Config::load(cfg_path)?;

    let logger = Logger::new(config.log_level.clone(), config.logs.clone(), config.syslog)?;

    if let Some(arg) = args {
        config.server_addr = arg.parse()?;
    }

    let (orchestrator, requests_tx) = Orchestrator::new(
        Services::new(config.get_includes().clone())?,
        logger.clone(),
    );

    thread::spawn(move || {
        orchestrator.orchestrate();
    });

    let listen_sock: TcpListener = TcpListener::bind(config.server_addr)?;
    loop {
        let sock_read: TcpStream = listen_sock.accept()?.0;
        let requests_tx = requests_tx.clone();
        let logger = logger.clone();

        let handle = thread::spawn(move || -> std::io::Result<()> {
            let mut deserializer = serde_json::Deserializer::from_reader(&sock_read);
            let mut sock_write: TcpStream = sock_read.try_clone()?;

            let mut sentinel = 0;

            loop {
                let req = Request::deserialize(&mut deserializer)?;
                logger::info!(logger, "Request: {:?}", req);
                let res: Response = process_request(&req);
                sock_write.write(serde_json::to_string(&res)?.as_bytes())?;

                let action = if sentinel % 2 == 0 {
                    ServiceAction::Start("ls1".to_string())
                } else {
                    // ServiceAction::Reload
                    ServiceAction::Stop("ls1".to_string())
                };

                sentinel += 1;

                // START
                let (cli_tx, cli_rx) = mpsc::channel();
                requests_tx
                    .send(orchestrate::OrchestratorMsg::Request(OrchestratorRequest {
                        action,
                        response_channel: cli_tx,
                    }))
                    .inspect_err(|err| {
                        logger::error!(logger, "Aending request to orchestrator! {err:?}")
                    })
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
