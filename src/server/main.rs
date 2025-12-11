mod config;
mod events;
mod jobs;
mod orchestrate;
mod service;
mod watcher;

use config::Config;
use logger::{LogLevel, Logger};
use orchestrate::{Orchestrator, OrchestratorMsg, OrchestratorRequest};
use serde_json::Deserializer;
use service::{ServiceAction, Services};
use std::{
    error::Error,
    io::Write,
    net::{TcpListener, TcpStream},
    sync::mpsc::{self, Sender},
    thread::{self},
};
use taskmeister::{Request, Response, ResponsePart, dir_utils};

fn command_to_action(req: &Request) -> Option<ServiceAction> {
    let alias = req.args.first().cloned().unwrap_or_default();

    match req.command.as_str() {
        "start" | "st" => Some(ServiceAction::Start(alias)),
        "stop" | "sp" => Some(ServiceAction::Stop(alias)),
        "restart" | "rs" => Some(ServiceAction::Restart(alias)),
        "reload" | "rl" => Some(ServiceAction::Reload),
        _ => None,
    }
}

// this is here for client testing purposes
fn process_request(
    req: &Request,
    requests_tx: Sender<OrchestratorMsg>,
    logger: Logger,
) -> Response {
    let (tx, rx) = mpsc::channel();
    let Some(action) = command_to_action(req) else {
        return vec![ResponsePart::Error(format!(
            "Command [{}] not found",
            req.command
        ))];
    };

    requests_tx
        .send(OrchestratorMsg::Request(OrchestratorRequest {
            action,
            response_channel: tx,
        }))
        .inspect_err(|err| logger::error!(logger, "Sending request to orchestrator {err}"))
        .ok();

    let mut response = vec![];
    for resp in rx {
        response.push(match resp {
            Ok(_) => ResponsePart::Info("OK".to_string()),
            Err(err) => ResponsePart::Error(err.to_string()),
        })
    }

    response
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
            let deserializer = Deserializer::from_reader(&sock_read).into_iter::<Request>();
            let mut sock_write: TcpStream = sock_read.try_clone()?;

            for req in deserializer {
                let Ok(req) = req else {
                    logger::warn!(logger, "Deserializing {req:?}");
                    continue;
                };
                logger::info!(logger, "{req:?}");
                let res = process_request(&req, requests_tx.clone(), logger.clone());
                sock_write.write(serde_json::to_string(&res)?.as_bytes())?;
            }

            Ok(())
        });

        let _ = handle.join().unwrap();
    }
}
