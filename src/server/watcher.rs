use std::{
    collections::HashMap,
    io,
    process::{Child, ExitStatus},
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::Duration,
};

use crate::jobs::JobStatus;

struct WatchedJob {
    process: Child,
    status: JobStatus,
    previous_status: JobStatus,
}

struct Watcher {
    watched: HashMap<String, Vec<WatchedJob>>,
    tx_events: Sender<JobStatus>,
}

impl Watcher {
    pub fn new() -> (Watcher, Receiver<JobStatus>) {
        let (tx, rx) = mpsc::channel();

        let new_instance = Watcher {
            watched: Vec::new(),
            tx_events: tx,
        };

        (new_instance, rx)
    }

    pub fn watch(&mut self, frequency: Duration) {
        loop {
            for job in &mut self.watched {
                let new_status = exit_status_to_job_status(job.process.try_wait());

                if new_status != job.previous_status {
                    job.previous_status = new_status.clone();

                    if let Err(e) = self.tx_events.send(new_status) {
                        eprintln!("Watcher send event error: {e}");
                    }
                }
            }
            thread::sleep(frequency);
        }
    }
}

fn exit_status_to_job_status(status: io::Result<Option<ExitStatus>>) -> JobStatus {
    match status {
        Ok(result) => match result {
            Some(exit_status) => JobStatus::Finished(exit_status),
            None => JobStatus::Running,
        },
        Err(_) => JobStatus::InternalError,
    }
}
