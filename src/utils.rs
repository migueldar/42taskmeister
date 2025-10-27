use std::{env, path::PathBuf};

// TODO: Dummy implementation, only parsing "-f" flag, extend this if
// needed more flags appart from "-f"
pub fn parse_config_path() -> (Option<PathBuf>, Option<String>) {
    let mut args = env::args().skip(1);
    while let Some(s) = args.next() {
        if s == "-f" {
            break;
        } else {
            return (None, Some(s));
        }
    }

    (args.next().map(|e| PathBuf::from(e)), args.next())
}

// Only needed for non-shell inputs
pub fn expand_home_dir(path: &str) -> PathBuf {
    if let Some(s) = path.strip_prefix("~/") {
        if let Some(mut home) = env::home_dir() {
            home.push(PathBuf::from(s));
            return home;
        }
    }
    PathBuf::from(path)
}
