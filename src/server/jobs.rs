use std::{
    collections::HashMap,
    fmt, io,
    process::ExitCode,
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
    last_exit_code: i32, // TODO: Check the need of this
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
    ServiceStopped,
    ServiceAlreadyStarted,
    ServiceAlreadyStopping,
    JobNotFound,
    JobIoError(io::Error),
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
    Finished(i32),
    TimedOut,
}

pub enum OrchestratorMsg {
    Request(OrchestratorRequest),
    Event(JobEvent),
}

pub struct Orchestrator {
    services: Services,
    jobs: HashMap<String, Job>,
    watched: Arc<Mutex<HashMap<String, Vec<WatchedJob>>>>,
    messages_tx: Sender<OrchestratorMsg>,
    messages_rx: Receiver<OrchestratorMsg>,
}

impl Orchestrator {
    pub fn new(services: Services) -> (Orchestrator, Sender<OrchestratorMsg>) {
        let (tx, rx) = mpsc::channel();

        (
            Orchestrator {
                services: services,
                jobs: HashMap::new(),
                watched: Arc::new(Mutex::new(HashMap::new())),
                messages_tx: tx.clone(),
                messages_rx: rx,
            },
            tx,
        )
    }

    // #################### UTILS ####################

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
            last_exit_code: 0,
        }))
    }

    fn start_job(&self, alias: String) -> Result<Option<Vec<WatchedJob>>, OrchestratorError> {
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
                    .map_err(|e| OrchestratorError::JobIoError(e))?,
                status: JobStatus::Starting,
                previous_status: JobStatus::Starting,
                timeout: WatchedTimeout {
                    created_at: Instant::now(),
                    time: Duration::from_secs(service.timeout),
                },
            }],
        ))
    }

    fn stop_job(&self, alias: String) -> Result<(), OrchestratorError> {
        let stop_signal = self
            .services
            .get(&alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?
            .stop_signal;

        // Get all the jobs id
        let job_pids: Vec<u32> = self
            .watched
            .lock()
            .unwrap()
            .get(&alias)
            .ok_or(OrchestratorError::JobNotFound)?
            .iter()
            .map(|watched_job| watched_job.process.id())
            .collect();

        // Kill the jobs
        for pid in job_pids {
            // TODO: Avoid early return?
            kill(pid, map_os_signal(stop_signal))
                .map_err(|err| OrchestratorError::JobIoError(err))?;
        }

        Ok(())
    }

    // #################### REQUESTS ####################
    fn start_request(&mut self, alias: String) -> Result<(), OrchestratorError> {
        // Get or create a new job
        let job = self.create_job(alias.clone())?;

        // Only finished, created and Free are considered valid states to start a job
        let mut response = match job.status {
            JobStatus::Starting
            | JobStatus::Running
            | JobStatus::Stopping
            | JobStatus::TimedOut => Err(OrchestratorError::ServiceAlreadyStarted),
            JobStatus::Finished(exit_status) => {
                // At this point event loop will have moved the job
                // out from the watcher
                job.last_exit_code = exit_status;
                Ok(())
            }
            JobStatus::Free | JobStatus::Created => Ok(()),
        };

        // Update job
        job.status = JobStatus::Starting;
        job.next_expected_status = Some(JobStatus::Running);

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

        // Only starting and Running are considered valid states to stop a job
        let mut response = match job.status {
            JobStatus::Starting | JobStatus::Running => Ok(()),
            JobStatus::Stopping | JobStatus::TimedOut => {
                Err(OrchestratorError::ServiceAlreadyStopping)
            }
            JobStatus::Finished(exit_status) => {
                // At this point event loop will have moved the job
                // out from the watcher
                job.last_exit_code = exit_status;
                Err(OrchestratorError::ServiceStopped)
            }
            JobStatus::Free | JobStatus::Created => Err(OrchestratorError::ServiceStopped),
        };

        // Update job
        job.status = JobStatus::Stopping;
        // TODO: Maybe Finished code should depend on the signal?
        job.next_expected_status = Some(JobStatus::Finished(0));

        // If response is positive, stop the job
        if let Ok(_) = response {
            response = self.stop_job(alias);
        };

        response
    }

    pub fn orchestrate(mut self) {
        let watched_jobs_thread = Arc::clone(&self.watched);
        let tx_events = self.messages_tx.clone();

        thread::spawn(move || {
            watcher::watch(watched_jobs_thread, tx_events, Duration::from_millis(10));
        });

        // NOTE: As designed orchestrate is only working with one sigle channel of requests.
        // If not job structure should be protecetd by mutex. This way only watched needs
        // protection since watcher also access the structure (in fact is the one
        // that consumes most of the lock time)

        // TODO: Put events and requests all in the one channel

        // TODO: Add a request to update services, maybe service should contain config

        while let Some(message) = self.messages_rx.iter().next() {
            match message {
                OrchestratorMsg::Request(request) => {
                    let result = match request.action {
                        ServiceAction::Start(alias) => self.start_request(alias),
                        ServiceAction::Restart(_) => todo!(),
                        ServiceAction::Stop(alias) => self.stop_request(alias),
                    };

                    request
                        .response_channel
                        .send(result)
                        .inspect_err(|err| eprintln!("Error: Sendig response to client: {err:?}"))
                        .ok();
                }
                OrchestratorMsg::Event(event) => todo!(),
            }
        }
    }
}

// #################### FILE UTILS ####################

fn kill(pid: u32, signal: OsSignal) -> io::Result<()> {
    if unsafe { libc::kill(pid as i32, signal.value()) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn map_os_signal(signal: i32) -> OsSignal {
    match signal {
        15 => OsSignal::SigTerm,
        _ => OsSignal::SigKill,
    }
}
