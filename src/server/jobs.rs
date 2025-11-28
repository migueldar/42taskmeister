// Note: not a submodule since it is just a semantical separation of the orchestrator
// module, but it is indeed the orchestrator and can not be splitted without having
// orchestrator depeendencies.

use libc;
use std::{io, time::Duration};

use crate::{
    orchestrate::{Orchestrator, OrchestratorError},
    watcher::{Watched, WatchedTimeout},
};

pub struct Job {
    pub status: JobStatus,
    pub retries: u8,
    pub last_exit_code: i32, // TODO: Check the need of this
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
    pub fn create_job(&mut self, alias: &str) -> Result<&mut Job, OrchestratorError> {
        // If job does not exist, create it
        Ok(self.jobs.entry(alias.to_string()).or_insert(Job {
            status: JobStatus::Created,
            retries: 0,
            last_exit_code: 0,
        }))
    }

    pub fn start_job(&self, alias: &str) -> Result<Option<Vec<Watched>>, OrchestratorError> {
        let service = self
            .get_services()
            .get(&alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        eprintln!("[{}] Starting", alias);
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
    pub fn stop_job(&self, alias: &str) -> Result<(), OrchestratorError> {
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
        eprintln!("[{}] Killing({})", alias, signal);

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
}

// #################### UTILS ####################

fn kill(pid: u32, signal: i32) -> io::Result<()> {
    if unsafe { libc::kill(pid as i32, signal) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
