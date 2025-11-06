use std::{
    collections::HashMap,
    fmt, io,
    process::{Child, ExitStatus},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::watcher::{self, WatchedJob};

use super::service::{Service, ServiceAction, Services};

struct Job {
    status: JobStatus,
    last_exit_status: ExitStatus, // TODO: Check the need of this
}

#[derive(Debug)]
enum JobAction {
    Start,
    Restart,
    Stop,
    Inform,
}

pub enum OrchestratorError {
    ServiceNotFound,
    ServiceAlreadyStarted,
    JobSpawnError(io::Error),
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Orchestrator error")
    }
}

#[derive(Debug)]
pub struct OrchestratorRequest {
    action: ServiceAction,
    response_channel: Sender<Result<(), OrchestratorError>>,
}

#[derive(PartialEq, Clone)]
pub enum JobStatus {
    Free,
    Starting,
    Running,
    Stopping,
    Finished(ExitStatus),
    InternalError,
}

pub fn orchestrate(services: Services, rx: Receiver<OrchestratorRequest>) {
    let mut jobs: HashMap<String, Job> = HashMap::new();
    let watched_jobs: Arc<Mutex<HashMap<String, Vec<WatchedJob>>>> =
        Arc::new(Mutex::new(HashMap::new()));
    let (tx_events, rx_events) = mpsc::channel();
    let watched_jobs_thread = Arc::clone(&watched_jobs);

    thread::spawn(move || {
        watcher::watch(watched_jobs_thread, tx_events, Duration::from_millis(10));
    });

    // TODO: There is a request channel for receiving client requests, the watcher will
    // send its events trough rx_events, must read from both channels without blocking
    // to manage events and requests

    // TODO: Extract all the above variables into a struct and create methods like
    // jobs.create() to make the code more readable

    for request in rx {
        match request.action {
            ServiceAction::Start(alias) => {
                let service = services.get(&alias).cloned();

                let Some(service) = service else {
                    request
                        .response_channel
                        .send(Err(OrchestratorError::ServiceNotFound));
                    continue;
                };

                // If job does not exist, start it
                let Some(job) = jobs.get_mut(&alias) else {
                    // First insert a job in Starting status
                    jobs.insert(
                        alias.clone(),
                        Job {
                            status: JobStatus::Starting,
                            last_exit_status: ExitStatus::default(),
                        },
                    );

                    // Spawn a job handler from service and return error if any
                    let handler = match service.start() {
                        Ok(handler) => handler,
                        Err(err) => {
                            request
                                .response_channel
                                .send(Err(OrchestratorError::JobSpawnError(err)));
                            continue;
                        }
                    };

                    // Add handler to the watched jobs
                    let mut watched = watched_jobs.lock().unwrap();
                    watched.insert(
                        alias,
                        vec![WatchedJob {
                            process: handler,
                            status: JobStatus::Starting,
                            previous_status: JobStatus::Starting,
                        }],
                    );

                    request.response_channel.send(Ok(()));

                    continue;
                };

                let response = match job.status {
                    JobStatus::Starting | JobStatus::Running | JobStatus::Stopping => {
                        Err(OrchestratorError::ServiceAlreadyStarted)
                    }
                    JobStatus::Finished(exit_status) => {
                        job.last_exit_status = exit_status;
                        Ok(())
                    }
                    JobStatus::Free => todo!(),
                    JobStatus::InternalError => todo!(),
                };
            }
            ServiceAction::Restart(alias) => todo!(),
            ServiceAction::Stop(alias) => todo!(),
        }
    }
}
