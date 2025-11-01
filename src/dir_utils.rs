use std::{
    env, fs, io,
    path::{Path, PathBuf},
};

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
pub fn expand_home_dir(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~/") {
        if let Some(mut home) = env::home_dir() {
            home.push(stripped);
            return home;
        }
    }
    PathBuf::from(path)
}

pub fn walk_dir<F>(path: PathBuf, f: &mut F) -> Result<(), io::Error>
where
    F: FnMut(PathBuf) -> Result<(), io::Error>,
{
    if path.is_file() {
        return f(path);
    } else if path.is_dir() {
        for dir_entry in fs::read_dir(path)? {
            walk_dir(dir_entry?.path(), f)?;
        }
    }
    Ok(())
}
