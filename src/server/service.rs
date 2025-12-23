use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::Entry},
    fs::{self},
    io,
    path::PathBuf,
    process::{Child, Command, Stdio},
};
use taskmeister::dir_utils;

/// Actions on services and the alias of that service
#[derive(Debug, Clone)]
pub enum ServiceAction {
    Start(String),
    Restart(String),
    Stop(String),
    Status(String),
    Attach(String),
    Reload,
    Help,
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq, Clone)]
#[serde(tag = "type", content = "retries")]
pub enum RestartOptions {
    #[default]
    Never,
    Always(u8),
    OnError(u8),
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq, Clone)]
pub struct Service {
    #[serde(skip)]
    pub file: PathBuf,
    alias: String,
    cmd: String,
    clone: u16,
    pub restart: RestartOptions,
    pub start_time: u64,
    pub stop_signal: i32,
    pub stop_wait: u64,
    exit_codes: Vec<i32>,
    pub stdout: String,
    pub stdin: String,
    pub stderr: String,
    variables: Vec<(String, String)>,
    working_dir: PathBuf,
    umask: u16,
}

// Cannot implement methods of foreign types, use struct wrapper to abstract it
#[derive(Debug)]
pub struct Services {
    paths: Vec<PathBuf>,
    services: HashMap<String, Service>,
}

impl Services {
    pub fn new(paths: Vec<PathBuf>) -> Result<Self, io::Error> {
        let services = load_services(&paths)?;

        Ok(Services { paths, services })
    }

    /// Update current Services with the new structure: new becomes the new services
    /// and a diff is returned with the services that changed
    pub fn update(&mut self) -> Result<Vec<ServiceAction>, io::Error> {
        let mut up = vec![];
        let mut new_services = load_services(&self.paths)?;

        for (alias, serv) in &new_services {
            match self.services.entry(alias.clone()) {
                Entry::Occupied(o) => {
                    // If self contains the entry, check if it changed
                    if *o.get() != *serv {
                        up.push(ServiceAction::Restart(alias.clone()));
                    }
                    self.services.remove(alias);
                }
                Entry::Vacant(_) => {
                    // TODO: Check if auto start, makes sense to not
                    // start new services found automaatically
                    // up.push(ServiceAction::Start(alias.clone()))
                    ()
                }
            };
        }

        // Remaining in self are the services to stop. This means this service
        // has disapeared, so also means to remove the service
        up.extend(
            self.services
                .iter()
                .map(|(alias, _)| ServiceAction::Stop(alias.clone())),
        );

        // Update self with the new entries
        new_services.extend(self.services.drain());
        self.services = new_services;

        Ok(up)
    }

    pub fn get(&self, alias: &str) -> Option<&Service> {
        self.services.get(alias)
    }

    pub fn remove(&mut self, alias: &str) -> Option<Service> {
        self.services.remove(alias)
    }
}

fn load_services(paths: &Vec<PathBuf>) -> Result<HashMap<String, Service>, io::Error> {
    let mut services = HashMap::new();

    for p in paths {
        let p = dir_utils::expand_home_dir(p);
        dir_utils::walk_dir(p, &mut |closure_p| {
            let Ok(mut s) = toml::from_str::<Service>(&fs::read_to_string(&closure_p)?) else {
                return Err(io::Error::other(format!(
                    "Couldn't deserialize: {}",
                    closure_p.display()
                )));
            };

            match services.entry(s.alias.clone()) {
                Entry::Occupied(o) => {
                    return Err(io::Error::other(format!(
                        "Alias {} redefined in: {}",
                        o.key(),
                        closure_p.display(),
                    )));
                }
                Entry::Vacant(v) => {
                    s.file = closure_p;
                    v.insert(s);
                    Ok(())
                }
            }
        })?;
    }

    Ok(services)
}

impl Service {
    pub fn start(&self) -> Result<Child, io::Error> {
        let mut args = self.cmd.split_ascii_whitespace();

        Command::new(
            args.next()
                .ok_or(io::Error::other("No command provided!"))?,
        )
        .args(args)
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .stderr(Stdio::piped())
        .current_dir(&self.working_dir)
        .spawn()
    }

    pub fn validate_exit_code(&self, exit_code: i32) -> bool {
        for code in &self.exit_codes {
            if exit_code == *code {
                return true;
            }
        }
        return false;
    }
}
