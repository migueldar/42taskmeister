use std::{
    collections::HashMap,
    fmt, io,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use libc;

use crate::watcher::{self, JobEvent, WatchedJob, WatchedTimeout};

use super::service::{ServiceAction, Services};

struct Job {
    status: JobStatus,
    retries: Option<u8>,
    last_exit_code: i32, // TODO: Check the need of this
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

    // #################### GET/SET UTILS ####################
    fn get_job_status(&self, alias: &str) -> Option<JobStatus> {
        self.jobs.get(alias).map(|job| job.status.clone())
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

            last_exit_code: 0,
        }))
    }

    fn remove_watched(&self, alias: &str) -> Option<Vec<WatchedJob>> {
        self.watched.lock().unwrap().remove(alias)
    }

    fn remove_watched_timeout(&self, alias: &str) {
        if let Some(watched_jobs) = self.watched.lock().unwrap().get_mut(alias) {
            for job in watched_jobs {
                job.timeout.remove();
            }
        }
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
                previous_status: JobStatus::Starting,
                timeout: WatchedTimeout::new(Some(Duration::from_secs(service.timeout))),
            }],
        ))
    }

    // Stops the job according to the signal specified in the service configuration
    fn stop_job(&self, alias: &str) -> Result<(), OrchestratorError> {
        let service = self
            .services
            .get(alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        self.kill_job(
            alias,
            service.stop_signal,
            Duration::from_secs(service.stop_wait),
        )
    }

    // Sends a specific stop signal to the job, setting the timeout for the process to stop
    fn kill_job(
        &self,
        alias: &str,
        signal: i32,
        timeout: Duration,
    ) -> Result<(), OrchestratorError> {
        // Get all the jobs id
        let job_pids: Vec<u32> = self
            .watched
            .lock()
            .unwrap()
            .get_mut(alias)
            .ok_or(OrchestratorError::JobNotFound)?
            .iter_mut()
            .map(|watched_job| {
                watched_job.timeout = WatchedTimeout::new(Some(timeout));
                watched_job.process.id()
            })
            .collect();

        // Kill the jobs
        for pid in job_pids {
            // TODO: Avoid early return?
            kill(pid, signal).map_err(|err| OrchestratorError::JobIoError(err))?;
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

        // If response is positive, stop the job
        if let Ok(_) = response {
            response = self.stop_job(&alias);
        };

        response
    }

    fn manage_event(&mut self, event: JobEvent) {
        println!("[{}] {:?}", event.alias, event.status);

        let Some(service) = self.services.get(&event.alias).cloned() else {
            return;
        };

        let Some(previous_status) = self.get_job_status(&event.alias) else {
            return;
        };

        let new_status = match event.status {
            JobStatus::Free | JobStatus::Created | JobStatus::Starting | JobStatus::Stopping => {
                event.status
            }
            JobStatus::Running => {
                // (prev: timeout, current: Running) => Only possibilty: Timed out stopping
                if previous_status == JobStatus::TimedOut {
                    // Return timeout here so in next timeout event is known that
                    // job wasn't stopped
                    JobStatus::TimedOut
                } else {
                    self.remove_watched_timeout(&event.alias);
                    event.status
                }
            }
            JobStatus::Finished(_) => {
                self.remove_watched(&event.alias);

                event.status
            }
            JobStatus::TimedOut => {
                // TODO: Next implement retries
                match previous_status {
                    JobStatus::TimedOut => {
                        // If job (not watched job) is in time out status, it means that
                        // we previously tried to stop or kill it
                        if let Err(err) = self.kill_job(
                            &event.alias,
                            libc::SIGKILL,
                            Duration::from_secs(service.stop_wait),
                        ) {
                            eprintln!("Error: Kill job: {err}");
                            return;
                        };
                    }
                    _ => {
                        // Gracefully stop
                        if let Err(err) = self.kill_job(
                            &event.alias,
                            service.stop_signal,
                            Duration::from_secs(service.stop_wait),
                        ) {
                            eprintln!("Error: Couldn't stop job gracefully: {err}");
                            return;
                        };
                    }
                }
                event.status
            }
        };

        // TODO: Check to extract job borrow by a getter/setter
        let Some(job) = self.jobs.get_mut(&event.alias) else {
            return;
        };

        job.status = new_status;
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
                        .inspect_err(|err| eprintln!("Error: Sending response to client: {err:?}"))
                        .ok();
                }
                OrchestratorMsg::Event(event) => self.manage_event(event),
            }
        }
    }
}

// #################### FILE UTILS ####################

fn kill(pid: u32, signal: i32) -> io::Result<()> {
    if unsafe { libc::kill(pid as i32, signal) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
