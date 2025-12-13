// Note: not a submodule since it is just a semantical separation of the orchestrator
// module, but it is indeed the orchestrator and can not be splitted without having
// orchestrator depeendencies.

use libc;
use logger::LogLevel;
use std::{io, time::Duration};

use crate::{
    orchestrate::{Orchestrator, OrchestratorError},
    watcher::{Watched, WatchedTimeout},
};

// Flags that are consumed upon use
#[derive(Clone)]
pub struct JobFlags {
    pub remove_service: bool, // Flag to remove service once the job finish
    pub restart_job: bool,    // Flag to restart job, only used for reload config
}

impl JobFlags {
    pub fn default() -> JobFlags {
        JobFlags {
            remove_service: false,
            restart_job: false,
        }
    }

    pub fn consume(&mut self) -> JobFlags {
        let old = self.clone();
        *self = JobFlags::default();
        old
    }
}

pub struct Job {
    pub status: JobStatus,
    pub retries: u8,
    pub last_exit_code: i32, // TODO: Check the need of this
    pub flags: JobFlags,
}

#[derive(PartialEq, Clone, Debug)]
pub enum JobStatus {
    Created,
    Starting,
    Running(bool), // While false job is not healthy
    Stopping,
    Finished(i32),
    TimedOut,
}

impl Orchestrator {
    /// Checks for a job identified by alias. If the job does not exist it is created,
    /// and the status will be set to `Created`. If the job exists it returns it.
    fn create_job(&mut self, alias: &str) -> Result<&mut Job, OrchestratorError> {
        // If job does not exist, create it
        Ok(self.jobs.entry(alias.to_string()).or_insert(Job {
            status: JobStatus::Created,
            retries: 0,
            last_exit_code: 0,
            flags: JobFlags::default(),
        }))
    }

    fn start_job(&self, alias: &str) -> Result<Option<Vec<Watched>>, OrchestratorError> {
        let service = self
            .get_services()
            .get(&alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        logger::info!(self.logger, "[{}] Starting", alias);
        // Add handler to the watched jobs
        let mut watched = self.watched.lock().unwrap();
        Ok(watched.insert(
            alias.to_string(),
            vec![Watched {
                process: service
                    .start()
                    .map_err(|e| OrchestratorError::JobIoError(e))?,
                previous_status: JobStatus::Starting,
                timeout: WatchedTimeout::new(Some(Duration::from_secs(service.start_time))),
            }],
        ))
    }

    // Stops the job according to the signal specified in the service configuration
    fn stop_job(&self, alias: &str) -> Result<(), OrchestratorError> {
        let service = self
            .get_services()
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
    pub fn kill_job(
        &self,
        alias: &str,
        signal: i32,
        timeout: Duration,
    ) -> Result<(), OrchestratorError> {
        logger::info!(self.logger, "[{}] Killing({})", alias, signal);

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
    pub fn start_request(&mut self, alias: &str) -> Result<(), OrchestratorError> {
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

        // If response is positive, start the job
        if let Ok(_) = response {
            // Update job
            job.status = JobStatus::Starting;

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

    pub fn stop_request(
        &mut self,
        alias: &str,
        remove_service: bool,
        restart_job: bool,
    ) -> Result<(), OrchestratorError> {
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

        // If response is positive, stop the job
        if let Ok(_) = response {
            // Update job
            job.status = JobStatus::Stopping;
            job.flags = JobFlags {
                remove_service,
                restart_job,
            };

            response = self.stop_job(&alias);
        };

        response
    }

    pub fn job_status(&self, alias: &str) -> Result<String, OrchestratorError> {
        // Get the job
        let job = self.jobs.get(alias).ok_or(OrchestratorError::JobNotFound)?;

        Ok(format!(
            "status: {:?}\nretries: {}",
            job.status, job.retries
        ))
    }
}

// #################### UTILS ####################

fn kill(pid: u32, signal: i32) -> io::Result<()> {
    if unsafe { libc::kill(pid as i32, signal) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
