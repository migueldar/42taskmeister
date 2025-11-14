use std::{
    collections::HashMap,
    fmt, io,
    process::ExitStatus,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use libc;

use crate::watcher::{self, JobEvent, WatchedJob, WatchedTimeout};

use super::service::{ServiceAction, Services};

struct Job {
    status: JobStatus,
    retries: Option<u8>,
    next_expected_status: Option<JobStatus>,
    last_exit_status: ExitStatus, // TODO: Check the need of this
}

enum OsSignal {
    SigTerm,
    SigKill,
}

impl OsSignal {
    pub fn value(&self) -> i32 {
        match self {
            OsSignal::SigTerm => libc::SIGTERM,
            OsSignal::SigKill => libc::SIGKILL,
        }
    }
}

#[derive(Debug)]
enum JobAction {
    Start,
    Restart,
    Stop,
    Inform,
}

#[derive(Debug)]
pub enum OrchestratorError {
    ServiceNotFound,
    ServiceAlreadyStarted,
    JobNotFound,
    JobSpawnError(io::Error),
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Orchestrator error")
    }
}

#[derive(Debug)]
pub struct OrchestratorRequest {
    pub action: ServiceAction,
    pub response_channel: Sender<Result<(), OrchestratorError>>,
}

#[derive(PartialEq, Clone, Debug)]
pub enum JobStatus {
    Free,
    Created,
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

    /// Checks for a job identified by alias. If the job does not exist it is created,
    /// and the status will be set to `Created`. If the job exists it returns it.
    fn create_job(&mut self, alias: String) -> Result<&mut Job, OrchestratorError> {
        let service = self
            .services
            .get(&alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        // If job does not exist, create it
        Ok(self.jobs.entry(alias.clone()).or_insert(Job {
            status: JobStatus::Created,
            retries: service.calc_retries(),
            next_expected_status: Some(JobStatus::Running),
            last_exit_status: ExitStatus::default(),
        }))
    }

    fn start_job(&mut self, alias: String) -> Result<Option<Vec<WatchedJob>>, OrchestratorError> {
        let service = self
            .services
            .get(&alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        // Add handler to the watched jobs
        let mut watched = self.watched.lock().unwrap();
        Ok(watched.insert(
            alias,
            vec![WatchedJob {
                process: service
                    .start()
                    .map_err(|e| OrchestratorError::JobSpawnError(e))?,
                status: JobStatus::Starting,
                previous_status: JobStatus::Starting,
                timeout: WatchedTimeout {
                    created_at: Instant::now(),
                    time: Duration::from_secs(service.timeout),
                },
            }],
        ))
    }

    fn start_request(&mut self, alias: String) -> Result<(), OrchestratorError> {
        // Get or create a new job
        let job = match self.create_job(alias.clone()) {
            Ok(job) => job,
            Err(err) => {
                return Err(err);
            }
        };

        // Only finished, created and Free are considered valid states to start a job
        let mut response = match job.status {
            JobStatus::Starting
            | JobStatus::Running
            | JobStatus::Stopping
            | JobStatus::TimedOut => Err(OrchestratorError::ServiceAlreadyStarted),
            JobStatus::Finished(exit_status) => {
                // At this point event loop will have moved the job
                // out from the watcher
                job.last_exit_status = exit_status;
                Ok(())
            }
            JobStatus::Free | JobStatus::Created => Ok(()),
        };

        // If response is positive, start the job
        if let Ok(_) = response {
            response = match self.start_job(alias) {
                Ok(res) => {
                    if let Some(old_watched_jobs) = res {
                        // TODO: Do something with old jobs in this case?
                        eprintln!("Warning: Started new jobs but old where not cleaned up from the watcher: {old_watched_jobs:?}");
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            };
        };

        response
    }

    fn stop_request(&mut self, alias: String) -> Result<(), OrchestratorError> {
        // Get the job
        let job = self
            .jobs
            .get_mut(&alias)
            .ok_or(OrchestratorError::JobNotFound)?;

        // TODO: COntinue here, decide where to use nix crate, or libc to send specific signal to stop a process

        // Only starting and Running are considered valid states to stop a job
        let mut response = match job.status {
            JobStatus::Starting | JobStatus::Running => Ok(()),
            JobStatus::Stopping | JobStatus::TimedOut => {
                Err(OrchestratorError::ServiceAlreadyStarted)
            }
            JobStatus::Finished(exit_status) => {
                // At this point event loop will have moved the job
                // out from the watcher
                job.last_exit_status = exit_status;
                Ok(())
            }
            JobStatus::Free | JobStatus::Created => Ok(()),
        };

        // If response is positive, start the job
        if let Ok(_) = response {
            response = match self.start_job(alias) {
                Ok(res) => {
                    if let Some(old_watched_jobs) = res {
                        // TODO: Do something with old jobs in this case?
                        eprintln!("Warning: Started new jobs but old where not cleaned up from the watcher: {old_watched_jobs:?}");
                    }
                    Ok(())
                }
                Err(err) => Err(err),
            };
        };

        response
    }

    pub fn orchestrate(mut self) {
        let (tx_events, _) = mpsc::channel();
        let watched_jobs_thread = Arc::clone(&self.watched);

        thread::spawn(move || {
            watcher::watch(watched_jobs_thread, tx_events, Duration::from_millis(10));
        });

        // TODO: There is a request channel for receiving client requests, the watcher will
        // send its events trough rx_events, must read from both channels without blocking
        // to manage events and requests

        // TODO: Extract all the above variables into a struct and create methods like
        // jobs.create() to make the code more readable

        // TODO: Put events and requests all in the one channel

        //TODO: Add a request to update services, maybe service should contain config

        while let Some(request) = self.requests.iter().next() {
            let result = match request.action {
                ServiceAction::Start(alias) => self.start_request(alias),
                ServiceAction::Restart(_) => todo!(),
                ServiceAction::Stop(_) => todo!(),
            };

            request
                .response_channel
                .send(result)
                .inspect_err(|err| eprintln!("Error: Sendig response to client: {err:?}"))
                .ok();
        }
    }
}

fn kill(pid: i32, signal: OsSignal) -> io::Result<()> {
    if unsafe { libc::kill(pid, signal.value()) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
