use std::{
    collections::HashMap,
    fmt,
    process::Child,
    sync::mpsc::{Receiver, Sender},
};

use super::service::{Service, ServiceAction, Services};

struct Job {
    process: Option<Child>,
    status: JobStatus,
}

#[derive(Debug)]
enum JobAction {
    Start,
    Restart,
    Stop,
    Inform,
}

#[derive(Debug)]
enum RequestType {
    ActionOnService(ServiceAction),
    ActionOnJob(JobAction),
}

pub enum OrchestratorError {
    ServiceNotFound,
}

impl fmt::Display for OrchestratorError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Orchestrator error")
    }
}

type OrchestratorResponse = Service;

#[derive(Debug)]
pub struct OrchestratorRequest {
    req_type: RequestType,
    response_channel: Sender<Result<OrchestratorResponse, OrchestratorError>>,
}

enum JobStatus {
    Starting,
    Started,
    Stopping,
    Free,
}

pub fn orchestrate(services: Services, rx: Receiver<OrchestratorRequest>) {
    let jobs: HashMap<String, Job> = HashMap::new();

    for request in rx {
        match request.req_type {
            RequestType::ActionOnService(service_action) => match service_action {
                ServiceAction::Start(alias) => {
                    let service = services
                        .get(&alias)
                        .cloned()
                        .ok_or(OrchestratorError::ServiceNotFound);

                    // If job does not exist, user is allowed to start the job
                    let Some(job) = jobs.get(&alias) else {
                        // First insert a job in Starting status
                        jobs.insert(
                            alias,
                            Job {
                                process: None,
                                status: JobStatus::Starting,
                            },
                        );

                        request.response_channel.send(service);

                        continue;
                    };

                    match job.status {
                        JobStatus::Starting => todo!(),
                        JobStatus::Started => todo!(),
                        JobStatus::Stopping => todo!(),
                        JobStatus::Free => {
                            job.status = JobStatus::Starting;
                            request.response_channel.send(service);
                        }
                    }
                }
                ServiceAction::Restart(alias) => todo!(),
                ServiceAction::Stop(alias) => todo!(),
            },
            RequestType::ActionOnJob(job_action) => todo!(),
            RequestType::Result => todo!(),
        }
    }
}
