use std::{
    collections::HashMap,
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    thread,
    time::Duration,
};

use logger::{LogLevel, Logger};

const READ_BUF_LEN: usize = 1024;

pub struct Tee {
    stdout: ChildStdout,
    // stdin: ChildStdin,
    // stderr: ChildStderr,
    def_stdout: Option<File>,
    // def_stdin: Stdio,
    // def_stderr: Stdio,
    tx: Option<SyncSender<Vec<u8>>>,
}

impl Tee {
    // Create a new tee with default values. This way the subsequent functions only
    // need to call to a give buffer and the default value. TODO: Check if the
    // default value is the only way we need to use this tee moudle.
    pub fn new(
        stdout: ChildStdout,
        // stdin: ChildStdin,
        // stderr: ChildStderr,
        def_stdout: &str,
        // def_stdin: &str,
        // def_stderr: &str,
    ) -> Result<Tee, io::Error> {
        Ok(Tee {
            stdout,
            // stdin,
            // stderr,
            def_stdout: match def_stdout {
                "null" => None,
                o => Some(OpenOptions::new().create(true).write(true).open(o)?),
            },
            // def_stdin: match def_stdin {
            //     "stdin" => Stdio::inherit(),
            //     "null" => Stdio::null(),
            //     i => Stdio::from(OpenOptions::new().create(true).read(true).open(i)?),
            // },
            // def_stderr: match def_stderr {
            //     "stderr" => Stdio::inherit(),
            //     "null" => Stdio::null(),
            //     o => Stdio::from(OpenOptions::new().create(true).write(true).open(o)?),
            // },
            tx: None,
        })
    }
}

pub enum IoRouterRequest {
    Create(String, ChildStdout, String), // Alias, Pipe, Default File
    Remove(String),                      // Alias
    StartForwarding((String, SyncSender<Vec<u8>>)), // Alias, Channel
    StopForwarding(String),              // Alias
}

pub fn route(requests: Receiver<IoRouterRequest>, logger: Logger) {
    let mut ios: HashMap<String, Tee> = HashMap::new();
    let period = Duration::from_millis(100);

    loop {
        while let Ok(req) = requests.try_recv() {
            match req {
                IoRouterRequest::StartForwarding((alias, resp_tx)) => {
                    if let Some(tee) = ios.get_mut(&alias) {
                        tee.tx = Some(resp_tx);
                    }
                }
                IoRouterRequest::StopForwarding(alias) => {
                    if let Some(tee) = ios.get_mut(&alias) {
                        tee.tx = None;
                    }
                }
                IoRouterRequest::Create(alias, stdout, default) => {
                    match Tee::new(stdout, &default) {
                        Ok(tee) => {
                            ios.entry(alias).or_insert(tee);
                        }
                        Err(err) => logger::warn!(logger, "Creating new Tee: {}", err),
                    }
                }

                IoRouterRequest::Remove(alias) => {
                    ios.remove(&alias);
                }
            }
        }

        for (_, tee) in &mut ios {
            let mut buf = [0; READ_BUF_LEN];

            match tee.stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(bytes) => {
                    if let Some(tx) = &tee.tx {
                        tx.send(buf[..bytes].to_vec());
                    }

                    if let Some(stdout) = &mut tee.def_stdout {
                        stdout.write(&buf[..bytes]);
                    }
                }
                Err(err) => {
                    logger::error!(logger, "Reading from stdout: {err}");
                    break;
                }
            }
        }
        thread::sleep(period);
    }
}
