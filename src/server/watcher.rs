use std::{
    collections::HashMap,
    io,
    process::{Child, ExitStatus},
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex,
    },
    thread,
    time::Duration,
};

use crate::jobs::JobStatus;

pub struct WatchedJob {
    process: Child,
    status: JobStatus,
    previous_status: JobStatus,
}

pub fn watch(
    watched_jobs: Arc<Mutex<HashMap<String, Vec<WatchedJob>>>>,
    tx_events: Sender<JobStatus>,
    frequency: Duration,
) {
    loop {
        let mut watched = watched_jobs.lock().unwrap();
        for (_, jobs) in watched.iter_mut() {
            for job in jobs {
                let new_status = exit_status_to_job_status(job.process.try_wait());

                if new_status != job.previous_status {
                    job.previous_status = new_status.clone();

                    if let Err(e) = tx_events.send(new_status) {
                        eprintln!("Watcher send event error: {e}");
                    }
                }
            }
        }
        thread::sleep(frequency);
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
