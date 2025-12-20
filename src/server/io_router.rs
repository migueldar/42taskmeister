use std::{
    collections::{HashMap, VecDeque},
    fs::{File, OpenOptions},
    io::{self, Read, Write},
    process::{ChildStderr, ChildStdin, ChildStdout},
    sync::mpsc::{self, Receiver, Sender, SyncSender},
    thread,
    time::Duration,
};

use logger::{LogLevel, Logger};

const READ_BUF_LEN: usize = 1024;
const DEQUE_BUF_LEN: usize = 10;

struct Stdout {
    pipe: ChildStdout,
    def_stdout: Option<File>,
    tx: Option<SyncSender<Vec<u8>>>,
    buff: VecDeque<Vec<u8>>,
}

impl Stdout {
    fn read(&mut self, buf: &mut [u8]) -> Result<(), io::Error> {
        match self.pipe.read(buf) {
            Ok(0) => Ok(()),
            Ok(bytes) => {
                // Always push into the ring buffer
                ring_buf_push(&mut self.buff, buf[..bytes].to_vec());

                if let Some(tx) = &self.tx {
                    let _ = tx.send(buf[..bytes].to_vec());
                }

                if let Some(stdout) = &mut self.def_stdout {
                    let _ = stdout.write(&buf[..bytes]);
                }

                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(err) => Err(err),
        }
    }
}

struct Stderr {
    pipe: ChildStderr,
    def_stderr: Option<File>,
    tx: Option<SyncSender<Vec<u8>>>,
    buff: VecDeque<Vec<u8>>,
}

impl Stderr {
    fn read(&mut self, buf: &mut [u8]) -> Result<(), io::Error> {
        match self.pipe.read(buf) {
            Ok(0) => Ok(()),
            Ok(bytes) => {
                // Always push into the ring buffer
                ring_buf_push(&mut self.buff, buf[..bytes].to_vec());

                if let Some(tx) = &self.tx {
                    let _ = tx.send(buf[..bytes].to_vec());
                }

                if let Some(stderr) = &mut self.def_stderr {
                    let _ = stderr.write(&buf[..bytes]);
                }

                Ok(())
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(err) => Err(err),
        }
    }
}

pub struct Tee {
    stdout: Stdout,
    stderr: Stderr,
    // stdin: ChildStdin,
    // def_stdin: Stdio,
}

impl Tee {
    // Create a new tee with default values. This way the subsequent functions only
    // need to call to a give buffer and the default value. TODO: Check if the
    // default value is the only way we need to use this tee moudle.
    pub fn new(
        stdout: ChildStdout,
        // stdin: ChildStdin,
        stderr: ChildStderr,
        def_stdout: &str,
        // def_stdin: &str,
        def_stderr: &str,
    ) -> Result<Tee, io::Error> {
        Ok(Tee {
            stdout: Stdout {
                pipe: stdout,
                def_stdout: match def_stdout {
                    "null" => None,
                    o => Some(OpenOptions::new().create(true).write(true).open(o)?),
                },
                tx: None,
                buff: VecDeque::with_capacity(READ_BUF_LEN * DEQUE_BUF_LEN),
            },
            stderr: Stderr {
                pipe: stderr,
                def_stderr: match def_stderr {
                    "null" => None,
                    o => Some(OpenOptions::new().create(true).write(true).open(o)?),
                },
                tx: None,
                buff: VecDeque::with_capacity(READ_BUF_LEN * DEQUE_BUF_LEN),
            }, // stdin,
               // stderr,

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
        })
    }
}

pub enum IoRouterRequest {
    Create(String, ChildStdout, ChildStderr, String, String), // Alias, Stdout Pipe, Stdout Pipe, Default Stdout File, Default Stderr File
    Remove(String),                                           // Alias
    ReadBuff(String, Sender<(Vec<u8>, Vec<u8>)>), // Alias, Stdout Channel, Stderr Channel
    StartForwardingStream(String, SyncSender<Vec<u8>>), // Alias, Channel
    StopForwarding(String),                       // Alias
}

pub fn route(requests: Receiver<IoRouterRequest>, logger: Logger) {
    let mut ios: HashMap<String, Tee> = HashMap::new();
    let period = Duration::from_millis(100);

    loop {
        while let Ok(req) = requests.try_recv() {
            match req {
                IoRouterRequest::StartForwardingStream(alias, resp_tx) => {
                    if let Some(tee) = ios.get_mut(&alias) {
                        tee.stdout.tx = Some(resp_tx);
                    }
                }
                IoRouterRequest::ReadBuff(alias, resp_tx) =>
                // Send one time a vector with the whole contents of the current buffer
                {
                    let _ = resp_tx.send(match ios.get(&alias) {
                        Some(tee) => (
                            tee.stdout
                                .buff
                                .iter()
                                .flat_map(|elem| elem.iter().cloned())
                                .collect(),
                            tee.stderr
                                .buff
                                .iter()
                                .flat_map(|elem| elem.iter().cloned())
                                .collect(),
                        ),
                        None => (Vec::new(), Vec::new()),
                    });
                }
                IoRouterRequest::StopForwarding(alias) => {
                    if let Some(tee) = ios.get_mut(&alias) {
                        tee.stdout.tx = None;
                        tee.stderr.tx = None;
                    }
                }
                IoRouterRequest::Create(alias, stdout, stderr, def_stdout, def_stderr) => {
                    match Tee::new(stdout, stderr, &def_stdout, &def_stderr) {
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

            tee.stdout
                .read(&mut buf)
                .inspect_err(|err| logger::error!(logger, "Reading from stdout: {err}"))
                .ok();
            tee.stderr
                .read(&mut buf)
                .inspect_err(|err| logger::error!(logger, "Reading from stderr: {err}"))
                .ok();
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
    fn read_buff(&self, alias: &str) -> (String, String);
    fn create(
        &self,
        alias: &str,
        stdout: ChildStdout,
        stderr: ChildStderr,
        def_stdout: &str,
        def_stderr: &str,
    );
    fn remove(&self, alias: &str);
}

impl RouterRequest for Sender<IoRouterRequest> {
    // Return Stdout and Stderr
    fn read_buff(&self, alias: &str) -> (String, String) {
        let (tx, rx) = mpsc::channel();

        if let Err(_) = self.send(IoRouterRequest::ReadBuff(alias.to_string(), tx)) {
            return (String::new(), String::new());
        }

        match rx.recv() {
            Ok((stdout_buf, stderr_buf)) => (
                String::from_utf8_lossy(&stdout_buf[stdout_buf.len().saturating_sub(512)..])
                    .to_string(),
                String::from_utf8_lossy(&stderr_buf[stderr_buf.len().saturating_sub(512)..])
                    .to_string(),
            ),
            Err(_) => (String::new(), String::new()),
        }
    }

    fn create(
        &self,
        alias: &str,
        stdout: ChildStdout,
        stderr: ChildStderr,
        def_stdout: &str,
        def_stderr: &str,
    ) {
        let _ = self.send(IoRouterRequest::Create(
            alias.to_string(),
            stdout,
            stderr,
            def_stdout.to_string(),
            def_stderr.to_string(),
        ));
    }

    fn remove(&self, alias: &str) {
        let _ = self.send(IoRouterRequest::Remove(alias.to_string()));
    }
}
