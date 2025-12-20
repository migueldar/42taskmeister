use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    thread,
    time::Duration,
};

use logger::{LogLevel, Logger};

const READ_BUF_LEN: usize = 1024;
const DEQUE_BUF_LEN: usize = 10;

pub struct Tee {
    stdout: ChildStdout,
    // stdin: ChildStdin,
    // stderr: ChildStderr,
    def_stdout: Option<File>,
    // def_stdin: Stdio,
    // def_stderr: Stdio,
    tx: Option<SyncSender<Vec<u8>>>,
    buff: VecDeque<Vec<u8>>,
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
            buff: VecDeque::with_capacity(READ_BUF_LEN * DEQUE_BUF_LEN),
        })
    }
}

pub enum IoRouterRequest {
    Create(String, ChildStdout, String), // Alias, Pipe, Default File
    Remove(String),                      // Alias
    ReadBuff(String, Sender<Vec<u8>>),   // Alias, Channel
    StartForwardingStream(String, SyncSender<Vec<u8>>), // Alias, Channel
    StopForwarding(String),              // Alias
}

pub fn route(requests: Receiver<IoRouterRequest>, logger: Logger) {
    let mut ios: HashMap<String, Tee> = HashMap::new();
    let period = Duration::from_millis(100);

    loop {
        while let Ok(req) = requests.try_recv() {
            match req {
                IoRouterRequest::StartForwardingStream(alias, resp_tx) => {
                    if let Some(tee) = ios.get_mut(&alias) {
                        tee.tx = Some(resp_tx);
                    }
                }
                IoRouterRequest::ReadBuff(alias, resp_tx) => {
                    // Send one time a vector with the whole contents of the current buffer
                    resp_tx.send(match ios.get(&alias) {
                        Some(tee) => tee
                            .buff
                            .iter()
                            .flat_map(|elem| elem.iter().cloned())
                            .collect(),
                        None => Vec::new(),
                    });
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
                    // Always push into the ring buffer
                    ring_buf_push(&mut tee.buff, buf[..bytes].to_vec());

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

fn ring_buf_push(buff: &mut VecDeque<Vec<u8>>, element: Vec<u8>) {
    if buff.len() == buff.capacity() {
        buff.pop_front();
    }

    buff.push_back(element);
}

pub trait RouterRequest {
    fn read_buff(&self, alias: &str) -> String;
}

impl RouterRequest for Sender<IoRouterRequest> {
    fn read_buff(&self, alias: &str) -> String {
        let (tx, rx) = mpsc::channel();

        if let Err(_) = self.send(IoRouterRequest::ReadBuff(alias.to_string(), tx)) {
            return String::new();
        }

        match rx.recv() {
            Ok(buf) => String::from_utf8_lossy(&buf[buf.len().saturating_sub(512)..]).to_string(),
            Err(_) => String::new(),
        }
    }
}
