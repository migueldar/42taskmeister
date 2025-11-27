use std::{
    collections::HashMap,
    io,
    process::{Child, ExitStatus},
    sync::{mpsc::Sender, Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crate::jobs::{JobStatus, OrchestratorMsg};

#[derive(Debug)]
pub struct JobEvent {
    pub alias: String,
    pub status: JobStatus,
}

#[derive(Debug)]
pub struct WatchedTimeout {
    created_at: Instant,
    time: Option<Duration>,
}

impl WatchedTimeout {
    pub fn has_timed_out(&self) -> bool {
        match self.time {
            Some(time) => self.created_at.elapsed() >= time,
            None => false,
        }
    }

    pub fn new(time: Option<Duration>) -> WatchedTimeout {
        WatchedTimeout {
            created_at: Instant::now(),
            time: time,
        }
    }

    pub fn remove(&mut self) {
        self.time = None
    }
}

#[derive(Debug)]
pub struct WatchedJob {
    pub process: Child,
    pub timeout: WatchedTimeout,
    pub previous_status: JobStatus,
}

pub fn watch(
    watched_jobs: Arc<Mutex<HashMap<String, Vec<WatchedJob>>>>,
    tx_events: Sender<OrchestratorMsg>,
    period: Duration,
) {
    loop {
        for (alias, jobs) in watched_jobs.lock().unwrap().iter_mut() {
            let event_sender = |event| {
                if let Err(e) = tx_events.send(OrchestratorMsg::Event(JobEvent {
                    alias: alias.clone(),
                    status: event,
                })) {
                    eprintln!("Watcher send event error: {e}");
                }
            };

            for job in jobs {
                if job.timeout.has_timed_out() {
                    if job.previous_status != JobStatus::TimedOut {
                        job.previous_status = JobStatus::TimedOut;
                        event_sender(JobStatus::TimedOut);
                    }
                    continue;
                }

                let new_status = exit_status_to_job_status(job.process.try_wait());
                if new_status != job.previous_status {
                    job.previous_status = new_status.clone();
                    event_sender(new_status);
                    continue;
                }
            }
        }
        thread::sleep(period);
    }
}

fn exit_status_to_job_status(status: io::Result<Option<ExitStatus>>) -> JobStatus {
    match status {
        Ok(result) => match result {
            Some(exit_status) => JobStatus::Finished(*exit_status.code().get_or_insert(0)),
            None => JobStatus::Running(true),
        },
        Err(_) => JobStatus::TimedOut,
    }
}
