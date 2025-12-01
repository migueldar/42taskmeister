use std::{
    io::{self, IsTerminal},
    time::{self, SystemTime},
};

const SECS_IN_DAY: u64 = 86400;
const DAYS_IN_ERA: u64 = 146097;

pub enum LogLevel {
    Info,
    Warning,
    Error,
}

pub struct Logger {
    level: LogLevel,
    is_term: bool,
}

impl Logger {
    pub fn new(level: LogLevel) -> Logger {
        Logger {
            level: level,
            is_term: io::stdout().is_terminal(),
        }
    }
    pub fn println_info(&self) {
        println!("{}", timestamp())
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

    format!("[{}/{}/{}]", day, month, year + (month <= 2) as u64)
}
