use std::{
    collections::HashMap,
    fmt, io,
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::Duration,
};

use logger::{LogLevel, Logger};

use crate::{
    events::JobEvent,
    jobs::{Job, JobStatus},
    service::{ServiceAction, Services},
    watcher::{self, Watched},
};

#[derive(Debug)]
pub enum OrchestratorError {
    ServiceNotFound,
    ServiceUpdate,
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

pub enum OrchestratorMsg {
    Request(OrchestratorRequest),
    Event(JobEvent),
}

pub struct Orchestrator {
    services: Services,
    pub logger: Logger,
    pub jobs: HashMap<String, Job>,
    pub watched: Arc<Mutex<HashMap<String, Vec<Watched>>>>,
    messages_tx: Sender<OrchestratorMsg>,
    messages_rx: Receiver<OrchestratorMsg>,
}

impl Orchestrator {
    pub fn new(services: Services, logger: Logger) -> (Orchestrator, Sender<OrchestratorMsg>) {
        let (tx, rx) = mpsc::channel();

        (
            Orchestrator {
                services,
                logger,
                jobs: HashMap::new(),
                watched: Arc::new(Mutex::new(HashMap::new())),
                messages_tx: tx.clone(),
                messages_rx: rx,
            },
            tx,
        )
    }

    // #################### GET/SET UTILS ####################
    pub fn get_job_status(&self, alias: &str) -> Option<JobStatus> {
        self.jobs.get(alias).map(|job| job.status.clone())
    }

    pub fn inc_job_retries(&mut self, alias: &str) -> Option<u8> {
        self.jobs.get_mut(alias).map(|job| {
            job.retries += 1;
            job.retries
        })
    }

    pub fn get_services(&self) -> &Services {
        &self.services
    }

    pub fn set_watched_status(
        &mut self,
        alias: &str,
        new_status: JobStatus,
    ) -> Result<(), OrchestratorError> {
        self.watched
            .lock()
            .unwrap()
            .get_mut(alias)
            .ok_or(OrchestratorError::JobNotFound)?
            .iter_mut()
            .for_each(|watched_job| watched_job.previous_status = new_status.clone());

        Ok(())
    }

    // #################### UTILS ####################

    pub fn remove_watched(&self, alias: &str) -> Option<Vec<Watched>> {
        self.watched.lock().unwrap().remove(alias)
    }

    pub fn remove_watched_timeout(&self, alias: &str) {
        if let Some(watched_jobs) = self.watched.lock().unwrap().get_mut(alias) {
            for job in watched_jobs {
                job.timeout.remove();
            }
        }
    }

    // #################### REQUESTS ####################
    fn start_request(&mut self, alias: &str) -> Result<(), OrchestratorError> {
        // Get or create a new job
        let job = self.create_job(alias)?;

        // Only finished, created and Free are considered valid states to start a job
        let mut response = match job.status {
            JobStatus::Starting
            | JobStatus::Running(_)
            | JobStatus::Stopping
            | JobStatus::TimedOut => Err(OrchestratorError::ServiceAlreadyStarted),
            JobStatus::Finished(exit_status) => {
                // At this point event loop will have moved the job
                // out from the watcher
                job.last_exit_code = exit_status;
                Ok(())
            }
            JobStatus::Created => Ok(()),
        };

        // Update job
        job.status = JobStatus::Starting;

        // If response is positive, start the job
        if let Ok(_) = response {
            response = match self.start_job(alias) {
                Ok(res) => {
                    if let Some(old_watched_jobs) = res {
                        // TODO: Do something with old jobs in this case?
                        logger::warn!(
                            self.logger,
                            "Started new jobs but old where not cleaned up from the watcher: {old_watched_jobs:?}"
                        );
                    }

                    Ok(())
                }
                Err(err) => Err(err),
            };
        };

        response
    }

    fn stop_request(&mut self, alias: &str) -> Result<(), OrchestratorError> {
        // Get the job
        let job = self
            .jobs
            .get_mut(alias)
            .ok_or(OrchestratorError::JobNotFound)?;

        // Only starting and Running are considered valid states to stop a job
        let mut response = match job.status {
            JobStatus::Starting | JobStatus::Running(_) => Ok(()),
            JobStatus::Stopping | JobStatus::TimedOut => {
                Err(OrchestratorError::ServiceAlreadyStopping)
            }
            JobStatus::Finished(exit_status) => {
                // At this point event loop will have moved the job
                // out from the watcher
                job.last_exit_code = exit_status;
                Err(OrchestratorError::ServiceStopped)
            }
            JobStatus::Created => Err(OrchestratorError::ServiceStopped),
        };

        // Update job
        job.status = JobStatus::Stopping;

        // If response is positive, stop the job
        if let Ok(_) = response {
            response = self.stop_job(&alias);
        };

        response
    }

    pub fn orchestrate(mut self) {
        let watched_jobs_thread = Arc::clone(&self.watched);
        let tx_events = self.messages_tx.clone();
        let wlogger = self.logger.clone();

        thread::spawn(move || {
            watcher::watch(
                watched_jobs_thread,
                tx_events,
                Duration::from_millis(2000),
                wlogger,
            );
        });

        // NOTE: By design orchestrate is only working with one sigle channel of requests.
        // If not job structure should be protecetd by mutex. This way only watched needs
        // protection since watcher also access the structure (in fact is the one
        // that consumes most of the lock time)

        // TODO: Add a request to update services, maybe service should contain config

        while let Some(message) = self.messages_rx.iter().next() {
            match message {
                OrchestratorMsg::Request(request) => {
                    let result = match request.action {
                        ServiceAction::Start(alias) => self.start_request(&alias),
                        ServiceAction::Restart(_) => todo!(),
                        ServiceAction::Stop(alias) => self.stop_request(&alias),
                        ServiceAction::Reload => match self.services.update() {
                            Ok(up_services) => {
                                let mut res = Ok(());
                                for service in up_services {
                                    res = match service {
                                        ServiceAction::Start(alias) => self.start_request(&alias),
                                        ServiceAction::Restart(_) => todo!(),
                                        ServiceAction::Stop(alias) => self.stop_request(&alias),
                                        _ => Ok(()),
                                    };

                                    if res.is_err() {
                                        break;
                                    }
                                }
                                res
                            }
                            Err(err) => {
                                logger::error!(self.logger, "Updating services: {err}");
                                Err(OrchestratorError::ServiceUpdate)
                            }
                        },
                    };

                    request
                        .response_channel
                        .send(result)
                        .inspect_err(|err| {
                            logger::error!(self.logger, "Sending response to client: {err}")
                        })
                        .ok();
                }
                OrchestratorMsg::Event(event) => self.manage_event(event),
            }
        }
    }
}
