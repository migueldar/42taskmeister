use serde::{Deserialize, Serialize};
use std::{
    collections::{hash_map::Entry, HashMap},
    fs, io,
    path::PathBuf,
};
use taskmeister::dir_utils;

/// Actions on services and the alias of that service
#[derive(Debug)]
pub enum ServiceAction {
    Start(String),
    Restart(String),
    Stop(String),
}

#[derive(Debug, Serialize, Deserialize, Default, PartialEq)]
#[serde(tag = "type", content = "value")]
enum RestartOptions {
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
    restart: RestartOptions,
    timeout: u32,
    stop_signal: u32,
    stop_wait: u32,
    // redirect: File,
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
        println!("Stopping {alias}");
        todo!()
    }

    pub fn remove(&mut self, alias: &str) -> Option<Service> {
        self.0.remove(alias)
    }
}
