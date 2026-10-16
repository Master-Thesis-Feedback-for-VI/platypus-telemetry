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
