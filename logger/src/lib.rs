use libc;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Write},
    path::PathBuf,
    sync::mpsc::{self, Receiver, Sender},
    thread,
    time::SystemTime,
};

#[macro_export]
macro_rules! info {
    ($logger:expr, $($arg:tt)*) => {
        $logger.send(LogLevel::Info, format!($($arg)*))
    };
}
#[macro_export]
macro_rules! warn {
    ($logger:expr, $($arg:tt)*) => {
        $logger.send(LogLevel::Warning, format!($($arg)*))
    };
}
#[macro_export]
macro_rules! error {
    ($logger:expr, $($arg:tt)*) => {
        $logger.send(LogLevel::Error, format!($($arg)*))
    };
}

const SECS_IN_DAY: u64 = 86400;
const DAYS_IN_ERA: u64 = 146097;

// This works since:
// https://doc.rust-lang.org/stable/std/cmp/trait.PartialOrd.html#derivable
#[derive(Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, Debug, Clone)]
pub enum LogLevel {
    Error,
    Warning,
    Info,
}

struct Log {
    level: LogLevel,
    msg: String,
}

#[derive(Clone)]
pub struct Logger {
    tx: Sender<Log>,
    syslog: bool,
}

impl Logger {
    pub fn new(level: LogLevel, logs_path: Option<PathBuf>, syslog: bool) -> io::Result<Self> {
        let mut file = None;

        if syslog {
            unsafe {
                libc::openlog(c"taskmaker".as_ptr(), libc::LOG_CONS, libc::LOG_USER);
            }
        }

        if let Some(logs_path) = logs_path {
            file = Some(File::options().create(true).append(true).open(logs_path)?);
        }

        let (tx, rx) = mpsc::channel();

        thread::spawn(move || {
            log_loop(rx, level, file, syslog);
        });

        Ok(Logger { tx, syslog })
    }

    pub fn send(&self, level: LogLevel, msg: String) {
        self.tx
            .send(Log { level, msg })
            .inspect_err(|err| eprintln!("Error: Sending to logger channel: {err}"))
            .ok();
    }
}

fn log_loop(rx: Receiver<Log>, level: LogLevel, mut file: Option<File>, syslog: bool) {
    for log in rx {
        let timestamp = timestamp();

        let (prefix, posix_level) = match level {
            LogLevel::Info => ("[INFO]", libc::LOG_INFO),
            LogLevel::Warning => ("[WARN]", libc::LOG_WARNING),
            LogLevel::Error => ("[ERROR]", libc::LOG_ERR),
        };

        if syslog {
            // In syslog we always log no matter the level
            unsafe {
                libc::syslog(posix_level, log.msg.as_ptr() as *const libc::c_char);
            }
        }

        // Only log levels with higher or equal severity as configured
        if log.level <= level {
            let log_msg = format!("{} {}: {}", timestamp, prefix, log.msg);
            println!("{log_msg}");

            if let Some(file) = &mut file {
                writeln!(file, "{log_msg}")
                    .inspect_err(|err| eprintln!("Error: {err}"))
                    .ok();
            }
        }
    }
}

impl Drop for Logger {
    fn drop(&mut self) {
        if self.syslog {
            unsafe {
                libc::closelog();
            }
        }
    }
}

// Calc timestamp using Howard Hinnant algorithm:
// https://howardhinnant.github.io/date_algorithms.html
fn timestamp() -> String {
    let Ok(now) = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) else {
        return "[Wrong System Time]".to_string();
    };

    let Some(days) = { now.as_secs() / SECS_IN_DAY }.checked_add(719468) else {
        return "[Time Overflow]".to_string();
    };

    // day/month/year in civil time
    let era = days / DAYS_IN_ERA;
    let day_of_era = days - era * DAYS_IN_ERA;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_non_civil = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_non_civil + 2) / 5 + 1;
    let month = if month_non_civil < 10 {
        month_non_civil + 3
    } else {
        month_non_civil - 9
    };

    // hour/minute/second in UTC
    let seconds_today = now.as_secs() % SECS_IN_DAY;
    let hour = seconds_today / 3600;
    let minute = (seconds_today - (hour * 3600)) / 60;
    let second = seconds_today - (hour * 3600) - (minute * 60);

    format!(
        "[{}/{}/{} {}:{}:{} UTC]",
        day,
        month,
        year + (month <= 2) as u64,
        hour,
        minute,
        second
    )
}
