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

use crate::orchestrate::OrchestratorError;

pub const IO_ROUTER_READ_BUF_LEN: usize = 1024;
const DEQUE_BUF_LEN: usize = 10;
const DRAIN_TIMES: usize = 100;

struct Stdout {
    pipe: ChildStdout,
    def_stdout: Option<File>,
    tx: Option<SyncSender<Vec<u8>>>,
    buff: VecDeque<Vec<u8>>,
}

impl Stdout {
    // Return Ok(false) if we do not want to continue reading
    fn forward(&mut self, buf: &mut [u8]) -> Result<bool, io::Error> {
        match self.pipe.read(buf) {
            Ok(0) => Ok(false),
            Ok(bytes) => {
                // Always push into the ring buffer
                ring_buf_push(&mut self.buff, buf[..bytes].to_vec());

                if let Some(tx) = &self.tx {
                    let _ = tx.send(buf[..bytes].to_vec());
                }

                if let Some(stdout) = &mut self.def_stdout {
                    let _ = stdout.write(&buf[..bytes]);
                }

                Ok(true)
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(false),
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
    // Return Ok(false) if we do not want to continue reading
    fn forward(&mut self, buf: &mut [u8]) -> Result<bool, io::Error> {
        match self.pipe.read(buf) {
            Ok(0) => Ok(false),
            Ok(bytes) => {
                // Always push into the ring buffer
                ring_buf_push(&mut self.buff, buf[..bytes].to_vec());

                if let Some(tx) = &self.tx {
                    let _ = tx.send(buf[..bytes].to_vec());
                }

                if let Some(stderr) = &mut self.def_stderr {
                    let _ = stderr.write(&buf[..bytes]);
                }

                Ok(true)
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => Ok(false),
            Err(err) => Err(err),
        }
    }
}

struct Stdin {
    pipe: ChildStdin,
    rx: Option<Receiver<Vec<u8>>>,
}

impl Stdin {
    fn forward(&mut self) -> Result<(), io::Error> {
        let Some(rx) = &self.rx else { return Ok(()) };

        if let Ok(data) = rx.try_recv() {
            if let Err(err) = self.pipe.write_all(&data) {
                return Err(err);
            }
        }

        Ok(())
    }
}

struct Tee {
    stdout: Stdout,
    stderr: Stderr,
}

impl Tee {
    // Create a new tee with default values. This way the subsequent functions only
    // need to call to a give buffer and the default value. TODO: Check if the
    // default value is the only way we need to use this tee moudle.
    fn new(
        stdout: ChildStdout,
        stderr: ChildStderr,
        def_stdout: &str,
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
                buff: VecDeque::with_capacity(IO_ROUTER_READ_BUF_LEN * DEQUE_BUF_LEN),
            },
            stderr: Stderr {
                pipe: stderr,
                def_stderr: match def_stderr {
                    "null" => None,
                    o => Some(OpenOptions::new().create(true).write(true).open(o)?),
                },
                tx: None,
                buff: VecDeque::with_capacity(IO_ROUTER_READ_BUF_LEN * DEQUE_BUF_LEN),
            },
        })
    }
}

pub enum IoRouterRequest {
    Create(String, ChildStdout, ChildStderr, String, String), // Alias, Stdout Pipe, Stderr Pipe, Stdin Pipe, Default Stdout File, Default Stderr File
    Remove(String),                                           // Alias
    ReadBuff(String, Sender<(Vec<u8>, Vec<u8>)>), // Alias, Stdout Channel, Stderr Channel
    StartForwarding(
        String,
        SyncSender<Vec<u8>>,
        SyncSender<Vec<u8>>,
        Sender<Result<(), OrchestratorError>>, // Receiver<Vec<u8>>,
    ), // Alias, Stdout Channel, Stderr Channel, Stdin Channel
    StopForwarding(String),                       // Alias
}

pub fn route(requests: Receiver<IoRouterRequest>, logger: Logger) {
    let mut ios: HashMap<String, Tee> = HashMap::new();
    let period = Duration::from_millis(100);
    let mut buff = [0; IO_ROUTER_READ_BUF_LEN];

    loop {
        while let Ok(req) = requests.try_recv() {
            match req {
                IoRouterRequest::StartForwarding(
                    alias,
                    stdout_channel,
                    stderr_channel,
                    resp_channel,
                ) => {
                    let result = resp_channel.send(if let Some(tee) = ios.get_mut(&alias) {
                        if matches!(tee.stdout.tx, Some(_)) {
                            Err(OrchestratorError::JobAlreadyAttached)
                        } else {
                            tee.stdout.tx = Some(stdout_channel);
                            tee.stderr.tx = Some(stderr_channel);
                            Ok(())
                        }
                        // tee.stdin.rx = Some(stdin_channel);
                    } else {
                        Err(OrchestratorError::JobNotFound)
                    });

                    if let Err(err) = result {
                        logger::error!(logger, "Sending to channel {err}");
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
                        if matches!(tee.stdout.tx, Some(_)) {
                            // First drain all the pipes up to times
                            let mut times = DRAIN_TIMES;
                            while tee.stdout.forward(&mut buff).unwrap_or(false) && times != 0 {
                                times -= 1;
                            }

                            let mut times = DRAIN_TIMES;
                            while tee.stderr.forward(&mut buff).unwrap_or(false) && times != 0 {
                                times -= 1;
                            }

                            // TODO: Drain stdin?

                            // Then remove the forward channel
                            tee.stdout.tx = None;
                            tee.stderr.tx = None;
                        }
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
            tee.stdout
                .forward(&mut buff)
                .inspect_err(|err| logger::error!(logger, "Reading from stdout: {err}"))
                .ok();
            tee.stderr
                .forward(&mut buff)
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
    fn start_forwarding(
        &self,
        alias: &str,
        stdout: SyncSender<Vec<u8>>,
        stderr: SyncSender<Vec<u8>>,
    ) -> Result<(), OrchestratorError>;
    fn stop_forwarding(&self, alias: &str) -> Result<(), OrchestratorError>;
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

    fn start_forwarding(
        &self,
        alias: &str,
        stdout: SyncSender<Vec<u8>>,
        stderr: SyncSender<Vec<u8>>,
    ) -> Result<(), OrchestratorError> {
        let (resp_tx, resp_rx) = mpsc::channel();

        self.send(IoRouterRequest::StartForwarding(
            alias.to_string(),
            stdout,
            stderr,
            resp_tx,
        ))
        .map_err(|_| OrchestratorError::InternalChannelSendError)?;

        resp_rx
            .recv()
            .unwrap_or(Err(OrchestratorError::InternalChannelReceiveError))
    }

    fn stop_forwarding(&self, alias: &str) -> Result<(), OrchestratorError> {
        self.send(IoRouterRequest::StopForwarding(alias.to_string()))
            .map_err(|_| OrchestratorError::InternalChannelSendError)
    }
}
