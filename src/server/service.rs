use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, hash_map::Entry},
    fs::{self, OpenOptions},
    io,
    path::PathBuf,
    process::{Child, Command, Stdio},
};
use taskmeister::dir_utils;

/// Actions on services and the alias of that service
#[derive(Debug)]
pub enum ServiceAction {
    Start(String),
    Restart(String),
    Stop(String),
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
    alias: String,
    cmd: String,
    clone: u16,
    pub restart: RestartOptions,
    pub start_time: u64,
    pub stop_signal: i32,
    pub stop_wait: u64,
    exit_codes: Vec<i32>,
    stdout: String,
    stdin: String,
    stderr: String,
    variables: Vec<(String, String)>,
    working_dir: PathBuf,
    umask: u16,
}

// Cannot implement methods of foreign types, use struct wrapper to abstract it
#[derive(Debug)]
pub struct Services(HashMap<String, Service>);

impl Services {
    pub fn load(paths: &Vec<PathBuf>) -> Result<Services, io::Error> {
        let mut servs: HashMap<String, Service> = HashMap::new();

        for p in paths {
            let p = dir_utils::expand_home_dir(p);
            dir_utils::walk_dir(p, &mut |closure_p| {
                let Ok(s) = toml::from_str::<Service>(&fs::read_to_string(&closure_p)?) else {
                    return Err(io::Error::other(format!(
                        "Couldn't deserialize: {}",
                        closure_p.display()
                    )));
                };

                match servs.entry(s.alias.clone()) {
                    Entry::Occupied(o) => {
                        return Err(io::Error::other(format!(
                            "Alias {} redefined in: {}",
                            o.key(),
                            closure_p.display(),
                        )));
                    }
                    Entry::Vacant(v) => {
                        v.insert(s);
                        Ok(())
                    }
                }
            })?;
        }
        Ok(Services(servs))
    }

    /// Update current Services with the new structure: new becomes the new services
    /// and a diff is returned with the services that changed
    pub fn update(&mut self, mut new: Services) -> Vec<ServiceAction> {
        let mut up = vec![];

        for (alias, serv) in &new.0 {
            match self.0.entry(alias.clone()) {
                Entry::Occupied(o) => {
                    // If self contains the entry, check if it changed
                    if *o.get() != *serv {
                        up.push(ServiceAction::Restart(alias.clone()));
                    }
                    self.0.remove(alias);
                }
                Entry::Vacant(_) => up.push(ServiceAction::Start(alias.clone())),
            };
        }

        // Remaining in self are the services to stop
        up.extend(
            self.0
                .iter()
                .map(|(alias, _)| ServiceAction::Stop(alias.clone())),
        );

        // Update self with the new entries
        new.0.extend(self.0.drain());
        *self = new;

        up
    }

    pub fn get(&self, alias: &str) -> Option<&Service> {
        self.0.get(alias)
    }

    pub fn stop(&mut self, alias: &str) {
        todo!()
    }

    pub fn remove(&mut self, alias: &str) -> Option<Service> {
        self.0.remove(alias)
    }
}

impl Service {
    pub fn start(&self) -> Result<Child, io::Error> {
        let stdout = match self.stdout.as_str() {
            "stdout" => Stdio::inherit(),
            "null" => Stdio::null(),
            o => Stdio::from(OpenOptions::new().create(true).write(true).open(o)?),
        };

        let stdin = match self.stdin.as_str() {
            "stdin" => Stdio::inherit(),
            "null" => Stdio::null(),
            i => Stdio::from(OpenOptions::new().create(true).read(true).open(i)?),
        };

        let stderr = match self.stderr.as_str() {
            "stderr" => Stdio::inherit(),
            "null" => Stdio::null(),
            o => Stdio::from(OpenOptions::new().create(true).write(true).open(o)?),
        };

        let mut args = self.cmd.split_ascii_whitespace();

        Command::new(
            args.next()
                .ok_or(io::Error::other("No command provided!"))?,
        )
        .args(args)
        .stdout(stdout)
        .stdin(stdin)
        .stderr(stderr)
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
