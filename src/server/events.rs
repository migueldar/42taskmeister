// Note: not a submodule since it is just a semantical separation of the orchestrator
// module, but it is indeed the orchestrator and can not be splitted without having
// orchestrator depeendencies.use std::time::Duration;

use crate::{
    io_router::RouterRequest, jobs::JobStatus, orchestrate::Orchestrator, service::RestartOptions,
};
use logger::LogLevel;
use std::time::Duration;

pub struct JobEvent {
    pub alias: String,
    pub status: JobStatus,
}

impl Orchestrator {
    pub fn manage_event(&mut self, event: JobEvent) {
        logger::info!(self.logger, "[Event] [{}] {}", event.alias, event.status);

        let Some(service) = self.get_services().get(&event.alias).cloned() else {
            return;
        };

        let Some(previous_status) = self.get_job_status(&event.alias) else {
            return;
        };

        let (new_status, restart) = match event.status {
            JobStatus::Created
            | JobStatus::Starting
            | JobStatus::Running(true)
            | JobStatus::Stopping => (event.status, false),

            JobStatus::Running(false) => {
                if matches!(previous_status, JobStatus::Running(true)) {
                    // If previous status was Running(true) it means it comes
                    // from a timeout which means it is healthy now
                    logger::info!(self.logger, "[{}] Healthy ✅", event.alias);
                    (previous_status, false)
                } else {
                    (event.status, false)
                }
            }

            JobStatus::Finished(exit_code) => 'finished: {
                // End the forwarding cleanly if any
                self.io_router_requests
                    .stop_forwarding(&event.alias)
                    .inspect_err(|err| logger::error!(self.logger, "Stop forwarding: {err}"))
                    .ok();

                // Remove the I/O handler
                self.io_router_requests.remove(&event.alias);

                // First remove the watched job
                self.remove_watched(&event.alias);

                // If job was stopping just end
                if previous_status == JobStatus::Stopping {
                    let flags = self.consume_job_flags(&event.alias);

                    if flags.remove_service {
                        self.remove_service(&event.alias);
                    }

                    if flags.restart_job {
                        break 'finished (event.status, true);
                    }

                    break 'finished (event.status, false);
                }

                // Restart if needed
                match service.restart {
                    RestartOptions::Never => (event.status, false),

                    RestartOptions::Always(retries) => 'status: {
                        if let Some(current_retries) = self.inc_job_retries(&event.alias) {
                            // Restrat if we didn't reach the maximum retries
                            if current_retries < retries {
                                break 'status (event.status, true);
                            }
                        }
                        logger::info!(self.logger, "[{}] Exhausted retries", &event.alias);
                        (event.status, false)
                    }

                    RestartOptions::Unexpected(retries) => 'status: {
                        if let Some(current_retries) = self.inc_job_retries(&event.alias) {
                            // Restrat if we didn't reach the maximum retries, and the code is not expected
                            if !service.validate_exit_code(exit_code) {
                                if current_retries < retries {
                                    break 'status (event.status, true);
                                }
                            } else {
                                break 'status (event.status, false);
                            }
                        }
                        logger::info!(self.logger, "[{}] Exhausted retries", &event.alias);
                        (event.status, false)
                    }
                }
            }

            JobStatus::TimedOut => {
                match previous_status {
                    JobStatus::Running(_) | JobStatus::Starting => {
                        // If it comes from running it means it is healthy now
                        self.remove_watched_timeout(&event.alias);
                        (JobStatus::Running(true), false)
                    }
                    JobStatus::TimedOut | JobStatus::Stopping => {
                        // If job (not watched job) is in stopping status, it means that
                        // we previously tried to stop or kill it
                        if let Err(err) = self.kill_job(
                            &event.alias,
                            libc::SIGKILL,
                            Duration::from_secs(service.stop_wait),
                        ) {
                            logger::error!(self.logger, "Kill job: {err}");
                        };
                        (JobStatus::Stopping, false)
                    }
                    _ => (JobStatus::TimedOut, false),
                }
            }
        };

        let Some(job) = self.jobs.get_mut(&event.alias) else {
            return;
        };

        job.status = new_status;

        // If job needs to be restarted, do it
        if restart {
            // If restart is true, the job is in finish, so  this will work
            if let Err(error) = self.start_request(&event.alias) {
                logger::error!(self.logger, "Restarting job: {error}");
            }
        }
    }
}
