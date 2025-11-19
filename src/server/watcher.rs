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
    pub created_at: Instant,
    pub time: Duration,
}

impl WatchedTimeout {
    pub fn has_timed_out(&self) -> bool {
        self.created_at.elapsed() >= self.time
    }

    pub fn new(time: Duration) -> WatchedTimeout {
        WatchedTimeout {
            created_at: Instant::now(),
            time: time,
        }
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
            for job in jobs {
                if job.timeout.has_timed_out() {
                    job.previous_status = JobStatus::TimedOut;

                    if let Err(e) = tx_events.send(OrchestratorMsg::Event(JobEvent {
                        alias: alias.clone(),
                        status: JobStatus::TimedOut,
                    })) {
                        eprintln!("Watcher send event error: {e}");
                    }

                    continue;
                }

                let new_status = exit_status_to_job_status(job.process.try_wait());
                if new_status != job.previous_status {
                    // println!("NEW STATUS: {new_status:#?}");
                    // println!("OLD STATUS: {:#?}", job.previous_status);

                    job.previous_status = new_status.clone();

                    if let Err(e) = tx_events.send(OrchestratorMsg::Event(JobEvent {
                        alias: alias.clone(),
                        status: new_status,
                    })) {
                        eprintln!("Watcher send event error: {e}");
                    }

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
            None => JobStatus::Running,
        },
        Err(_) => JobStatus::TimedOut,
    }
}
