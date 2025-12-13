use crate::{
    CLI_HELP,
    events::JobEvent,
    jobs::{Job, JobFlags, JobStatus},
    service::{Service, ServiceAction, Services},
    watcher::{self, Watched},
};
use logger::{LogLevel, Logger};
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
        match self {
            OrchestratorError::ServiceNotFound => write!(f, "Service not found"),
            OrchestratorError::ServiceUpdate => write!(f, "While updating services"),
            OrchestratorError::ServiceStopped => write!(f, "Stopping service"),
            OrchestratorError::ServiceAlreadyStarted => write!(f, "Service already started"),
            OrchestratorError::ServiceAlreadyStopping => write!(f, "Service already stopped"),
            OrchestratorError::JobNotFound => write!(f, "Job not found"),
            OrchestratorError::JobIoError(error) => write!(f, "Job I/O error: {}", error),
        }
    }
}

#[derive(Debug)]
pub struct OrchestratorRequest {
    pub action: ServiceAction,
    pub response_channel: Sender<Result<String, OrchestratorError>>,
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

    pub fn consume_job_flags(&mut self, alias: &str) -> JobFlags {
        let Some(job) = self.jobs.get_mut(alias) else {
            return JobFlags::default();
        };

        job.flags.consume()
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

    pub fn remove_service(&mut self, alias: &str) -> Option<Service> {
        self.services.remove(alias)
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
                        ServiceAction::Start(alias) => {
                            self.start_request(&alias).map(|_| "OK".to_string())
                        }
                        ServiceAction::Restart(alias) => self
                            .stop_request(&alias, false, true)
                            .map(|_| "OK".to_string()),
                        ServiceAction::Stop(alias) => self
                            .stop_request(&alias, false, false)
                            .map(|_| "OK".to_string()),
                        ServiceAction::Reload => match self.services.update() {
                            Ok(up_services) => {
                                let mut res = Ok(());
                                for service in up_services {
                                    res = match service {
                                        ServiceAction::Start(alias) => self.start_request(&alias),
                                        ServiceAction::Restart(alias) => {
                                            self.stop_request(&alias, true, true)
                                        }
                                        ServiceAction::Stop(alias) => {
                                            self.stop_request(&alias, true, false)
                                        }
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
                        }
                        .map(|_| "OK".to_string()),
                        ServiceAction::Status(alias) => self.job_status(&alias),
                        ServiceAction::Help => Ok(CLI_HELP.to_string()),
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
