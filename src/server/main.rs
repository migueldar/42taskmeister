mod config;
mod events;
mod io_router;
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
    io::{self, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread::{self},
};
use taskmeister::{Request, ResponsePart, dir_utils};

pub const CLI_HELP: &str = r#"Commands:
	start [st]	Start a service
	stop [sp]	Stop a job
	restart [rs]	Restart a job
	status [stat]	Show the current status of a job
	attach [at]	Attach the job to the current client
	detach [dt] 	Detach the job from every client
	reload [rl]	Reload the configuration for the services
	list [ls]	List all loaded services
	quit [q]	Exit client
	stop_server	Stop the server
	help [?]	Show this help
"#;

static SIGHUP_FLAG: AtomicBool = AtomicBool::new(false);

extern "C" fn interrupt_handler(_: libc::c_int) {
    SIGHUP_FLAG.store(true, Ordering::SeqCst);
}

fn sighup_reload_config_init(requests: Sender<OrchestratorMsg>, logger: Logger) {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = interrupt_handler as usize;
        libc::sigemptyset(&mut sa.sa_mask);
        libc::sigaction(libc::SIGHUP, &sa, std::ptr::null_mut());
    }

    thread::spawn(move || {
        loop {
            if SIGHUP_FLAG.swap(false, Ordering::SeqCst) {
                let (tx, rx) = mpsc::channel();

                logger::info!(logger, "SIGHUP handler: Reloading Configuration");

                if let Err(err) = requests.send(OrchestratorMsg::Request(OrchestratorRequest {
                    action: ServiceAction::Reload,
                    response_channel: tx,
                })) {
                    logger::error!(logger, "SIGHUP handler: {err}");
                }

                for response in rx {
                    logger::info!(
                        logger,
                        "SIGHUP handler: {}",
                        match response {
                            ResponsePart::Error(err) => err.to_string(),
                            ResponsePart::Info(resp) => resp,
                            ResponsePart::Stream(_) => "Wrong Response: Stream".to_string(),
                        }
                    )
                }
            }
        }
    });
}

fn command_to_action(req: Request) -> Option<ServiceAction> {
    let alias = req.args.first().cloned().unwrap_or_default();

    if let Some(input) = req.stream {
        return Some(ServiceAction::Input(alias, input));
    }

    match req.command.as_str() {
        "start" | "st" => Some(ServiceAction::Start(alias)),
        "stop" | "sp" => Some(ServiceAction::Stop(alias)),
        "restart" | "rs" => Some(ServiceAction::Restart(alias)),
        "status" | "stat" => Some(ServiceAction::Status(alias)),
        "attach" | "at" => Some(ServiceAction::Attach(alias)),
        "detach" | "dt" => Some(ServiceAction::Detach(alias)),
        "reload" | "rl" => Some(ServiceAction::Reload),
        "list" | "ls" => Some(ServiceAction::List),
        "help" | "?" => Some(ServiceAction::Help),
        "stop_server" => std::process::exit(0),
        _ => None,
    }
}

fn process_request(
    req: Request,
    requests_tx: Sender<OrchestratorMsg>,
    mut socket_tx: TcpStream,
) -> Result<(), Box<dyn Error>> {
    let Some(action) = command_to_action(req) else {
        socket_tx.write(
            serde_json::to_string(&[ResponsePart::Error(format!("Command not found",))])?
                .as_bytes(),
        )?;
        return Ok(());
    };

    let (tx, rx) = mpsc::channel();

    requests_tx.send(OrchestratorMsg::Request(OrchestratorRequest {
        action: action.clone(),
        response_channel: tx,
    }))?;

    match action {
        ServiceAction::Attach(_) => {
            thread::spawn(move || -> io::Result<()> {
                for data in rx {
                    socket_tx.write(serde_json::to_string(&[data])?.as_bytes())?;
                }
                Ok(())
            });
        }
        _ => {
            let response: Vec<ResponsePart> = rx.iter().collect();

            if response.len() > 0 {
                socket_tx.write(serde_json::to_string(&response)?.as_bytes())?;
            }
        }
    }

    Ok(())
}

fn startup_services(
    services: &Vec<String>,
    requests_tx: Sender<OrchestratorMsg>,
) -> Result<(), Box<dyn Error>> {
    for service in services {
        let (tx, rx) = mpsc::channel();
        requests_tx.send(OrchestratorMsg::Request(OrchestratorRequest {
            action: ServiceAction::Start(service.to_owned()),
            response_channel: tx,
        }))?;

        for response in rx {
            if let ResponsePart::Error(err) = response {
                return Err(std::io::Error::new(
                    io::ErrorKind::Other,
                    format!("Starting [{service}]: {err}"),
                )
                .into());
            }
        }
    }

    Ok(())
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

    // TODO: manage clean exit by taking the handle
    thread::spawn(move || {
        orchestrator.orchestrate();
    });

    // Start the services in init
    startup_services(&config.start.services, requests_tx.clone())?;

    // Handle sighup signal
    sighup_reload_config_init(requests_tx.clone(), logger.clone());

    let listen_sock: TcpListener = TcpListener::bind(config.server_addr)?;
    let mut handlers = Vec::new();
    loop {
        let sock_read: TcpStream = listen_sock.accept()?.0;
        let requests_tx = requests_tx.clone();
        let logger = logger.clone();

        let handle = thread::spawn(move || -> io::Result<()> {
            let deserializer = Deserializer::from_reader(&sock_read).into_iter::<Request>();

            for req in deserializer {
                let Ok(req) = req else {
                    logger::warn!(logger, "Deserializing {req:?}");
                    continue;
                };

                logger::info!(logger, "{req:?}");

                if let Err(err) = process_request(req, requests_tx.clone(), sock_read.try_clone()?)
                {
                    logger::error!(logger, "Processing request: {err}");
                }
            }

            Ok(())
        });

        handlers.push(handle);
    }
}
