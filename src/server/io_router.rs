use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    process::{ChildStderr, ChildStdin, ChildStdout, Stdio},
    thread,
    time::Duration,
};

pub struct Tee {
    stdout: ChildStdout,
    stdin: ChildStdin,
    stderr: ChildStderr,
    def_stdout: Stdio,
    def_stdin: Stdio,
    def_stderr: Stdio,
}

impl Tee {
    // Create a new tee with default values. This way the subsequent functions only
    // need to call to a give buffer and the default value. TODO: Check if the
    // default value is the only way we need to use this tee moudle.
    pub fn new(
        stdout: ChildStdout,
        stdin: ChildStdin,
        stderr: ChildStderr,
        def_stdout: &str,
        def_stdin: &str,
        def_stderr: &str,
    ) -> Result<Tee, io::Error> {
        Ok(Tee {
            stdout,
            stdin,
            stderr,
            def_stdout: match def_stdout {
                "stdout" => Stdio::inherit(),
                "null" => Stdio::null(),
                o => Stdio::from(OpenOptions::new().create(true).write(true).open(o)?),
            },
            def_stdin: match def_stdin {
                "stdin" => Stdio::inherit(),
                "null" => Stdio::null(),
                i => Stdio::from(OpenOptions::new().create(true).read(true).open(i)?),
            },
            def_stderr: match def_stderr {
                "stderr" => Stdio::inherit(),
                "null" => Stdio::null(),
                o => Stdio::from(OpenOptions::new().create(true).write(true).open(o)?),
            },
        })
    }
    pub fn stdout(&self, buff: impl Write) {}
    pub fn stderr(&self, buff: impl Write) {}
    pub fn stdin(&self, buff: impl Read) {}
}

struct IoRouter {
    io_set: Vec<Tee>,
    period: Duration,
}

// Io Router will loop infinitelly, in each iteration will write to the standard i/o present in Tee, then will write to each mpsc channel also given in each tee upon creation. Implement ring buffer and read prediocally to it. Then client will send trough the channel another channel to consume this buffer when needed.
impl IoRouter {
    pub fn route(&self) {
        loop {
            for io in &self.io_set {}
            thread::sleep(self.period);
        }
    }
}
