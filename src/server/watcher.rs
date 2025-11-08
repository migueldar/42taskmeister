use std::{
    collections::HashMap,
    io,
    process::{Child, ExitStatus},
    sync::{mpsc::Sender, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::jobs::JobStatus;

pub struct JobEvent {
    alias: String,
    status: JobStatus,
}

#[derive(Debug)]
pub struct WatchedTimeout {
    pub created_at: Instant,
    pub time: Duration,
}

impl WatchedTimeout {
    pub fn has_timed_out(&self) -> bool {
        self.created_at.elapsed() >= self.time
    }
}

#[derive(Debug)]
pub struct WatchedJob {
    pub process: Child,
    pub timeout: WatchedTimeout,
    pub status: JobStatus,
    pub previous_status: JobStatus,
}

pub fn watch(
    watched_jobs: Arc<Mutex<HashMap<String, Vec<WatchedJob>>>>,
    tx_events: Sender<JobEvent>,
    period: Duration,
) {
    loop {
        for (alias, jobs) in watched_jobs.lock().unwrap().iter_mut() {
            for job in jobs {
                let new_status = exit_status_to_job_status(job.process.try_wait());
                if new_status != job.previous_status {
                    // println!("NEW STATUS: {new_status:#?}");
                    // println!("OLD STATUS: {:#?}", job.previous_status);

                    job.previous_status = new_status.clone();

                    if let Err(e) = tx_events.send(JobEvent {
                        alias: alias.clone(),
                        status: new_status,
                    }) {
                        eprintln!("Watcher send event error: {e}");
                    }

                    continue;
                }

                if job.timeout.has_timed_out() {
                    if let Err(e) = tx_events.send(JobEvent {
                        alias: alias.clone(),
                        status: JobStatus::TimedOut,
                    }) {
                        eprintln!("Watcher send event error: {e}");
                    }
                }
            }
        }
        thread::sleep(period);
    }
}

fn exit_status_to_job_status(status: io::Result<Option<ExitStatus>>) -> JobStatus {
    match status {
        Ok(result) => match result {
            Some(exit_status) => JobStatus::Finished(exit_status),
            None => JobStatus::Running,
        },
        Err(_) => JobStatus::TimedOut,
    }
}
