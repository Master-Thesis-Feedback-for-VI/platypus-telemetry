use std::{
    fs::{self, File, OpenOptions},
    io::{self},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub credentials: Credentials,
    pub server_settings: ServerSettings,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Credentials {
    pub username: String,
    pub passwd: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ServerSettings {
    pub uri: String,
    pub topics: Box<[String]>,
}

pub const ENV_FILE: &str = include_str!("../.env.toml");

pub(crate) fn config() -> Result<Config, toml::de::Error> {
    let conf: Config = toml::from_str(ENV_FILE)?;
    Ok(conf)
}

pub(crate) fn create_log_file(file_name: &str, timestamp: bool) -> io::Result<File> {
    let file_name = participant_file_name(file_name)?;
    let log_name = if timestamp {
        let time = chrono::Local::now().format("%Y-%m-%d_%Hh%Mm").to_string();
        format!("{time}_{file_name}.log")
    } else {
        format!("{file_name}.log")
    };
    let dir_name: PathBuf = Path::new(".").join("logs");
    fs::create_dir_all(&dir_name)?;

    let file_path = dir_name.join(log_name);
    OpenOptions::new().create(true).append(true).open(file_path)
}

pub(crate) fn participant_file_name(name: &str) -> io::Result<String> {
    let name = name.trim();
    if name.is_empty()
        || name
            .chars()
            .any(|ch| ch.is_control() || matches!(ch, '/' | '\\'))
        || matches!(name, "." | "..")
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "Enter a participant name without path separators or control characters",
        ));
    }
    Ok(name.replace(' ', "_"))
}

#[cfg(test)]
mod tests {
    use super::participant_file_name;

    #[test]
    fn participant_names_become_safe_file_names() {
        assert_eq!(participant_file_name("  Jane Doe  ").unwrap(), "Jane_Doe");
        assert_eq!(participant_file_name("Zoë Smith").unwrap(), "Zoë_Smith");
        for name in ["", "  ", "../test", "a/b", "a\\b", ".", "..", "a\nb"] {
            assert!(participant_file_name(name).is_err(), "{name:?}");
        }
    }
}
