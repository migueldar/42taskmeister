use std::{
    collections::HashMap,
    fmt, io,
    process::{Child, ExitStatus},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use crate::watcher::{self, JobEvent, WatchedJob, WatchedTimeout};

use super::service::{Service, ServiceAction, Services};

struct Job {
    status: JobStatus,
    retries: Option<u8>,
    next_expected_status: Option<JobStatus>,
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
    TimedOut,
}

pub struct Orchestrator {
    services: Services,
    jobs: HashMap<String, Job>,
    watched: Arc<Mutex<HashMap<String, Vec<WatchedJob>>>>,
    events_channel: (Sender<JobEvent>, Receiver<JobEvent>),
    requests: Receiver<OrchestratorRequest>,
}

impl Orchestrator {
    pub fn new(services: Services) -> (Orchestrator, Sender<OrchestratorRequest>) {
        let (general_tx, general_rx) = mpsc::channel();

        (
            Orchestrator {
                services: services,
                jobs: HashMap::new(),
                watched: Arc::new(Mutex::new(HashMap::new())),
                events_channel: mpsc::channel(),
                requests: general_rx,
            },
            general_tx,
        )
    }

    pub fn orchestrate(mut self) {
        let (tx_events, rx_events) = mpsc::channel();
        let watched_jobs_thread = Arc::clone(&self.watched);

        thread::spawn(move || {
            watcher::watch(watched_jobs_thread, tx_events, Duration::from_millis(10));
        });

        // TODO: There is a request channel for receiving client requests, the watcher will
        // send its events trough rx_events, must read from both channels without blocking
        // to manage events and requests

        // TODO: Extract all the above variables into a struct and create methods like
        // jobs.create() to make the code more readable

        for request in self.requests {
            match request.action {
                ServiceAction::Start(alias) => {
                    let service = self.services.get(&alias).cloned();

                    let Some(service) = service else {
                        request
                            .response_channel
                            .send(Err(OrchestratorError::ServiceNotFound));
                        continue;
                    };

                    // If job does not exist, start it
                    let Some(job) = self.jobs.get_mut(&alias) else {
                        // First insert a job in Starting status
                        self.jobs.insert(
                            alias.clone(),
                            Job {
                                status: JobStatus::Starting,
                                retries: service.calc_timeout(),
                                next_expected_status: Some(JobStatus::Running),
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
                        let mut watched = self.watched.lock().unwrap();
                        watched.insert(
                            alias,
                            vec![WatchedJob {
                                process: handler,
                                status: JobStatus::Starting,
                                previous_status: JobStatus::Starting,
                                timeout: WatchedTimeout {
                                    created_at: Instant::now(),
                                    time: Duration::from_secs(service.timeout),
                                },
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
                        JobStatus::TimedOut => todo!(),
                    };
                }
                ServiceAction::Restart(alias) => todo!(),
                ServiceAction::Stop(alias) => todo!(),
            }
        }
    }
}
