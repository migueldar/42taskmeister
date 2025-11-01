use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::Entry, HashMap},
    fs::{self, OpenOptions},
    io,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::mpsc::Sender,
};
use taskmeister::utils;

/// Actions on services and the alias of that service
#[derive(Debug)]
pub enum Update {
    Start(String),
    Reload(String),
    Stop(String),
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type", content = "value")]
enum Restart {
    #[default]
    Never,
    Always(u8),
    OnError(u8),
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq)]
pub struct Service {
    alias: String,
    cmd: String,
    clone: u16,
    restart: Restart,
    timeout: u32,
    stop_signal: u32,
    stop_wait: u32,
    stdout: String,
    stdin: String,
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
            let p = utils::expand_home_dir(p);
            utils::walk_dir(p, &mut |closure_p| {
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
    pub fn update(&mut self, mut new: Services) -> Vec<Update> {
        let mut up = vec![];

        for (alias, serv) in &new.0 {
            match self.0.entry(alias.clone()) {
                Entry::Occupied(o) => {
                    // If self contains the entry, check if it changed
                    if *o.get() != *serv {
                        up.push(Update::Reload(alias.clone()));
                    }
                    self.0.remove(alias);
                }
                Entry::Vacant(_) => up.push(Update::Start(alias.clone())),
            };
        }

        // Remaining in self are the services to stop
        up.extend(self.0.iter().map(|(alias, _)| Update::Stop(alias.clone())));

        // Update self with the new entries
        new.0.extend(self.0.drain());
        *self = new;

        up
    }

    pub fn get(&self, alias: &str) -> Option<&Service> {
        self.0.get(alias)
    }

    pub fn stop(&mut self, alias: &str) -> Option<Service> {
        // TODO: implement
        self.0.remove(alias)
    }

    pub fn start(&self, alias: &str, tx: Sender<Child>) -> Result<(), io::Error> {
        let Some(service) = self.get(alias) else {
            return Err(io::Error::other(format!("Couldn't find {alias} service")));
        };

        let stdout = match service.stdout.as_str() {
            "stdout" => Stdio::inherit(),
            "null" => Stdio::null(),
            o => Stdio::from(OpenOptions::new().create(true).write(true).open(o)?),
        };

        let stdin = match service.stdin.as_str() {
            "stdin" => Stdio::inherit(),
            "null" => Stdio::null(),
            i => Stdio::from(OpenOptions::new().create(true).read(true).open(i)?),
        };

        let mut args = service.cmd.split_ascii_whitespace();

        println!("ARGS: {:#?}", args.clone().collect::<Vec<_>>());

        let handler = Command::new(
            args.next()
                .ok_or(io::Error::other("No command provided!"))?,
        )
        .args(args)
        .stdout(stdout)
        .stdin(stdin)
        .current_dir(&service.working_dir)
        .spawn();

        println!("Spawning command {handler:#?}");

        tx.send(handler?).unwrap();

        Ok(())
    }
}
