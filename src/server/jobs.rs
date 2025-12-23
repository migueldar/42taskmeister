// Note: not a submodule since it is just a semantical separation of the orchestrator
// module, but it is indeed the orchestrator and can not be splitted without having
// orchestrator depeendencies.

use libc;
use logger::{self, LogLevel};
use std::{
    io,
    os::fd::AsRawFd,
    sync::mpsc::{self, Sender},
    thread,
    time::Duration,
};
use taskmeister::ResponsePart;

use crate::{
    io_router::{self, RouterRequest},
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
    pub started: Option<String>,
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
            started: None,
        }))
    }

    fn start_job(&mut self, alias: &str) -> Result<Option<Vec<Watched>>, OrchestratorError> {
        let service = self
            .get_services()
            .get(&alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        logger::info!(self.logger, "[{}] Starting", alias);

        // Start the child process
        let mut child = service
            .start()
            .map_err(|e| OrchestratorError::JobIoError(e))?;

        let stdout = child
            .stdout
            .take()
            .ok_or(OrchestratorError::JobHasNoIoHandle)?;
        set_fd_non_blocking(&stdout);

        let stderr = child
            .stderr
            .take()
            .ok_or(OrchestratorError::JobHasNoIoHandle)?;
        set_fd_non_blocking(&stderr);

        let stdin = child
            .stdin
            .take()
            .ok_or(OrchestratorError::JobHasNoIoHandle)?;

        // Create an I/O handler
        self.io_router_requests.create(
            alias,
            stdout,
            stderr,
            stdin,
            &service.stdout,
            &service.stderr,
        );

        // Add handler to the watched jobs
        let mut watched = self.watched.lock().unwrap();
        Ok(watched.insert(
            alias.to_string(),
            vec![Watched {
                process: child,
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

        // Get all the jobs id and restart timeout
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
            response = match self.start_job(alias) {
                Ok(res) => {
                    self.set_job_status(alias, JobStatus::Starting);
                    self.set_job_timestamp(alias);

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
        let service = self
            .get_services()
            .get(alias)
            .cloned()
            .ok_or(OrchestratorError::ServiceNotFound)?;

        let (stdout, stderr) = self.io_router_requests.read_buff(alias);

        Ok(format!(
            r#"status: {:?} Since {}
PIDs: {}
Configuration: {}
Stdout:

{}

Stderr:

{}"#,
            job.status,
            job.started.as_ref().map_or("[]", |s| s),
            self.get_pid(alias)
                .unwrap_or(Vec::new())
                .iter()
                .map(|pid| pid.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            service.file.display(),
            stdout,
            stderr,
        ))
    }

    pub fn attach_job(&self, alias: &str, tx: Sender<ResponsePart>) {
        let (router_tx, router_rx) = mpsc::sync_channel(io_router::IO_ROUTER_READ_BUF_LEN);
        let logger = self.logger.clone();
        let io_router_requests = self.io_router_requests.clone();
        let alias = alias.to_string();

        io_router_requests.start_forwarding(&alias, router_tx.clone(), router_tx);

        thread::spawn(move || {
            let result =
                router_rx
                    .iter()
                    .find_map(|data| match tx.send(ResponsePart::Stream(data)) {
                        Ok(_) => None,
                        Err(err) => Some(err),
                    });

            // Check the result
            if let Some(err) = result {
                logger::error!(logger, "Streaming: {err}");
                let _ = tx.send(ResponsePart::Error(err.to_string()));
            } else {
                let _ = tx.send(ResponsePart::Info("OK".to_string()));
            }

            io_router_requests.stop_forwarding(&alias);
        });
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

fn set_fd_non_blocking<T>(fd: &T)
where
    T: AsRawFd,
{
    unsafe {
        let fd = fd.as_raw_fd();
        let flags = libc::fcntl(fd, libc::F_GETFL);
        libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
    }
}
