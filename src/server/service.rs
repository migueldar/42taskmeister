use serde::{Deserialize, Deserializer, Serialize, de};
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
    Detach(String),
    Input(String, Vec<u8>),
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
    numprocs: u16,
    pub restart: RestartOptions,
    pub start_time: u64,
    #[serde(deserialize_with = "deserialize_signal")]
    pub stop_signal: i32,
    pub stop_wait: u64,
    exit_codes: Vec<i32>,
    pub stdout: String,
    pub stdin: String,
    pub stderr: String,
    env: HashMap<String, String>,
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
            let Ok(mut service) = toml::from_str::<Service>(&fs::read_to_string(&closure_p)?)
            else {
                return Err(io::Error::other(format!(
                    "Couldn't deserialize: {}",
                    closure_p.display()
                )));
            };

            match services.entry(service.alias.clone()) {
                Entry::Occupied(o) => {
                    return Err(io::Error::other(format!(
                        "Alias {} redefined in: {}",
                        o.key(),
                        closure_p.display(),
                    )));
                }
                Entry::Vacant(v) => {
                    service.file = closure_p;
                    v.insert(service);
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
        .envs(&self.env)
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

// UTILS

fn signal_from_str(signal_string: &str) -> Option<i32> {
    let name = signal_string.trim().to_ascii_uppercase();
    let name = name.strip_prefix("SIG").unwrap_or(&name);

    match name {
        "HUP" => Some(libc::SIGHUP),
        "INT" => Some(libc::SIGINT),
        "QUIT" => Some(libc::SIGQUIT),
        "ILL" => Some(libc::SIGILL),
        "TRAP" => Some(libc::SIGTRAP),
        "ABRT" => Some(libc::SIGABRT),
        "BUS" => Some(libc::SIGBUS),
        "FPE" => Some(libc::SIGFPE),
        "KILL" => Some(libc::SIGKILL),
        "USR1" => Some(libc::SIGUSR1),
        "SEGV" => Some(libc::SIGSEGV),
        "USR2" => Some(libc::SIGUSR2),
        "PIPE" => Some(libc::SIGPIPE),
        "ALRM" => Some(libc::SIGALRM),
        "TERM" => Some(libc::SIGTERM),
        "CHLD" => Some(libc::SIGCHLD),
        "CONT" => Some(libc::SIGCONT),
        "STOP" => Some(libc::SIGSTOP),
        "TSTP" => Some(libc::SIGTSTP),
        "TTIN" => Some(libc::SIGTTIN),
        "TTOU" => Some(libc::SIGTTOU),
        _ => None,
    }
}

fn deserialize_signal<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    let string = String::deserialize(deserializer)?;

    signal_from_str(&string)
        .ok_or_else(|| de::Error::custom(format!("Invalid Signal name: {string}")))
}
