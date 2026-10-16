pub mod environment;

use std::{
    error::Error,
    fmt::Display,
    io::{self, Seek, SeekFrom, Write},
    sync::{Arc, Mutex},
    time::Duration,
};

use chrono::Utc;
use clap::Parser;
use mqtt5::{ConnectOptions, ConnectionEvent, MqttClient, SubscribeOptions};
use serde::{Deserialize, Serialize};

use crate::environment::{config, create_log_file};

#[derive(Parser, Debug)]
struct Args {
    protocol_name: String,
    #[arg(short, long)]
    time_stamp: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let config = config().expect(".env.toml missing");
    let credentials = &config.credentials;

    let options = ConnectOptions::new("telemetry-listener")
        .with_clean_start(true)
        .with_keep_alive(Duration::from_secs(30))
        .with_credentials(&credentials.username, &credentials.passwd)
        .with_automatic_reconnect(true)
        .with_reconnect_delay(Duration::from_secs(5), Duration::from_secs(120));

    let client = MqttClient::with_options(options);
    client
        .on_connection_event(|event| match event {
            ConnectionEvent::Reconnecting { attempt } => {
                eprintln!("reconnecting (attempt {attempt})");
            }
            other => eprintln!("connection event: {other:?}"),
        })
        .await?;

    client.connect(&config.server_settings.uri).await?;
    let file = create_log_file(&args.protocol_name, args.time_stamp)
        .map(|val| Arc::new(Mutex::new(val)))?;

    for topic in &config.server_settings.topics {
        let options = SubscribeOptions {
            qos: mqtt5::QoS::AtMostOnce,
            ..Default::default()
        };

        let f = Arc::clone(&file);
        let current_topic = topic.clone();
        client
            .subscribe_with_options(topic, options, move |msg| {
                let mut f = f.lock().expect("Log mutex poisoned");

                println!("Received data from: {current_topic}");
                if let Err(err) = write_to_file(&msg.payload, &current_topic, &mut *f) {
                    eprintln!("Failed to write log: {err}");
                }
            })
            .await?;
    }

    tokio::signal::ctrl_c().await?;
    client.disconnect().await?;
    {
        let f = file.lock().expect("Lock poisoned");
        f.sync_all()?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum DeviceType {
    IOsApp,
    LillyGo,
    GalaxyWatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LogData<'a> {
    device: DeviceType,
    tag: &'a str,
    data: &'a str,
}

trait IntoLogData<'a> {
    fn into_log_data(self) -> Result<LogData<'a>, serde_json::Error>;
}

impl<'a> IntoLogData<'a> for LogData<'a> {
    fn into_log_data(self) -> Result<Self, serde_json::Error> {
        Ok(self)
    }
}

impl<'a, T: AsRef<[u8]> + ?Sized> IntoLogData<'a> for &'a T {
    fn into_log_data(self) -> Result<LogData<'a>, serde_json::Error> {
        serde_json::from_slice(AsRef::<[u8]>::as_ref(self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WriteLogData<'a> {
    #[serde(borrow)]
    log: LogData<'a>,
    timestamp: chrono::DateTime<Utc>,
    topic: &'a str,
}

impl Display for WriteLogData<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let current_date = self.timestamp.format("%Y-%m-%d %H:%M");
        let device = self.log.device;
        let topic = self.topic;
        let tag = self.log.tag;
        let data = self.log.data;
        write!(
            f,
            "[{current_date} || {device:?} || {topic} || {tag}]:\t{data}"
        )
    }
}

fn write_to_file<'a>(
    payload: impl IntoLogData<'a>,
    topic: &str,
    mut writer: impl Write + Seek,
) -> io::Result<()> {
    let Ok(incoming_data) = payload.into_log_data() else {
        return Ok(());
    };

    let data = WriteLogData {
        log: incoming_data,
        timestamp: Utc::now(),
        topic,
    };

    writer.seek(SeekFrom::End(0))?;
    writeln!(writer, "{data}")
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::{DeviceType, LogData, write_to_file};

    #[test]
    fn write_to_log() {
        let mut buff = Cursor::new(Vec::with_capacity(1024));
        let log_data = LogData {
            device: DeviceType::IOsApp,
            tag: "ImageFlow",
            data: "Detected 10 cars",
        };
        write_to_file(log_data, "test/data", &mut buff).unwrap();

        let log_data = LogData {
            device: DeviceType::LillyGo,
            tag: "Vibrate",
            data: "brimmm",
        };
        write_to_file(log_data, "test/data", &mut buff).unwrap();

        let res = String::from_utf8(buff.into_inner()).unwrap();
        panic!("{res}")
    }
}
