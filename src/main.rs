pub mod environment;
pub mod shared_data;
mod ui;

use std::{
    error::{self, Error},
    fmt::Display,
    io::{self, ErrorKind, Read, Seek, SeekFrom, Write},
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

use chrono::{ParseResult, Utc};
use clap::Parser;
use mqtt5::{ConnectOptions, ConnectionEvent, MqttClient, SubscribeOptions};
use serde::{Deserialize, Serialize};

use crate::{
    environment::{config, create_log_file},
    shared_data::{DeviceType, LogData},
};

#[derive(Parser, Debug)]
struct Args {
    #[arg(short, long)]
    time_stamp: bool,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let mut terminal = ui::TerminalSession::new()?;
    let Some(participant) = ui::ask_participant(&mut terminal.terminal)? else {
        return Ok(());
    };
    let file = Arc::new(Mutex::new(create_log_file(&participant, args.time_stamp)?));
    let config = config()?;
    let credentials = &config.credentials;

    let options = ConnectOptions::new("telemetry-listener")
        .with_clean_start(true)
        .with_keep_alive(Duration::from_secs(30))
        .with_credentials(&credentials.username, &credentials.passwd)
        .with_automatic_reconnect(true)
        .with_reconnect_delay(Duration::from_secs(5), Duration::from_secs(120));

    let client = MqttClient::with_options(options);
    let (messages, receiver) = mpsc::channel();
    let connection_messages = messages.clone();
    client
        .on_connection_event(move |event| {
            let message = match event {
                ConnectionEvent::Reconnecting { attempt } => {
                    format!("Reconnecting (attempt {attempt})")
                }
                other => format!("Connection event: {other:?}"),
            };
            let _ = connection_messages.send(message);
        })
        .await?;

    let mqtt_client = client.clone();
    let mqtt_file = Arc::clone(&file);
    let connection = tokio::spawn(async move {
        if let Err(error) = subscribe(&mqtt_client, &config, &mqtt_file, &messages).await {
            let _ = messages.send(format!("MQTT error: {error}"));
        }
    });

    let result = ui::run(&mut terminal.terminal, &participant, &file, &receiver);
    drop(terminal);
    connection.abort();
    let _ = connection.await;
    let disconnect = tokio::time::timeout(Duration::from_secs(3), client.disconnect()).await;
    file.lock()
        .map_err(|_| io::Error::other("Log mutex poisoned"))?
        .sync_all()?;
    result?;
    if let Ok(result) = disconnect {
        result?;
    }
    Ok(())
}

async fn subscribe(
    client: &MqttClient,
    config: &environment::Config,
    file: &Arc<Mutex<std::fs::File>>,
    messages: &mpsc::Sender<String>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    client.connect(&config.server_settings.uri).await?;
    for topic in &config.server_settings.topics {
        let options = SubscribeOptions {
            qos: mqtt5::QoS::AtMostOnce,
            ..Default::default()
        };

        let f = Arc::clone(file);
        let current_topic = topic.clone();
        let messages = messages.clone();
        client
            .subscribe_with_options(topic, options, move |msg| {
                let result = f
                    .lock()
                    .map_err(|_| io::Error::other("Log mutex poisoned"))
                    .and_then(|mut f| write_to_file(&msg.payload, &current_topic, &mut *f));
                let message = match result {
                    Ok(written_log) => written_log.to_string(),
                    Err(err) => format!("Failed to write log: {err}"),
                };
                let _ = messages.send(message);
            })
            .await?;
    }

    Ok(())
}

trait IntoLogData {
    fn into_log_data(self) -> Result<LogData, serde_json::Error>;
}

impl IntoLogData for LogData {
    fn into_log_data(self) -> Result<Self, serde_json::Error> {
        Ok(self)
    }
}

impl<T: AsRef<[u8]> + ?Sized> IntoLogData for &T {
    fn into_log_data(self) -> Result<LogData, serde_json::Error> {
        serde_json::from_slice(AsRef::<[u8]>::as_ref(self))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct WriteLogData {
    log: LogData,
    timestamp: chrono::DateTime<Utc>,
    topic: Box<str>,
}

impl WriteLogData {
    pub(crate) fn serialize<W: Write>(&self, writer: &mut W) -> io::Result<()> {
        let data = self.to_string();
        write!(writer, "{data}")
    }

    pub(crate) fn deserialize<R: Read>(reader: &mut R) -> io::Result<Self> {
        let metadata = Self::read_metadata(reader)?;
        let mut metadata_values = metadata.split("||").map(str::trim);

        let current_date_repr = metadata_values
            .next()
            .ok_or_else(|| Self::invalid_metadata_value("Could not parse date"))?;
        let current_date =
            Self::date_into_chrono(current_date_repr).map_err(Self::invalid_metadata_value)?;

        let device_repr = metadata_values
            .next()
            .ok_or_else(|| Self::invalid_metadata_value("Could not parse date"))?;
        let device: DeviceType = device_repr.parse().map_err(Self::invalid_metadata_value)?;

        let topic = metadata_values
            .next()
            .ok_or_else(|| Self::invalid_metadata_value("Could not parse date"))?
            .into();

        let tag = metadata_values
            .next()
            .ok_or_else(|| Self::invalid_metadata_value("Could not parse date"))?
            .into();

        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        if byte[0] != b':' {
            return Err(Self::invalid_metadata_value("Expected ':'"));
        }

        let mut data_buf = Vec::new();
        reader.read_to_end(&mut data_buf)?;
        let data = String::from_utf8(data_buf)
            .map_err(Self::invalid_metadata_value)?
            .trim()
            .into();

        let log_data = LogData { device, tag, data };
        Ok(Self {
            log: log_data,
            timestamp: current_date,
            topic,
        })
    }

    fn read_metadata<R: Read>(reader: &mut R) -> io::Result<String> {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;

        if byte[0] != b'[' {
            return Err(io::Error::new(ErrorKind::InvalidData, "Expected ["));
        }

        let mut metadata = Vec::new();
        loop {
            reader.read_exact(&mut byte)?;

            if byte[0] == b']' {
                break;
            }

            if metadata.len() > 4096 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Metadata too large",
                ));
            }

            metadata.push(byte[0]);
        }

        String::from_utf8(metadata).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))
    }

    fn date_into_chrono(date: &str) -> ParseResult<chrono::DateTime<Utc>> {
        chrono::DateTime::parse_from_rfc3339(date).map(|dt| dt.with_timezone(&Utc))
    }

    fn invalid_metadata_value<E>(reason: E) -> io::Error
    where
        E: Into<Box<dyn error::Error + Send + Sync>>,
    {
        io::Error::new(ErrorKind::InvalidData, reason)
    }
}

impl Display for WriteLogData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let current_date = self.timestamp.to_rfc3339();
        let device = self.log.device;
        let topic = self.topic.as_ref();
        let tag = self.log.tag.as_ref();
        let data = self.log.data.as_ref();
        write!(
            f,
            "[ {current_date} || {device:?} || {topic} || {tag} ]:\t{data}"
        )
    }
}

fn write_to_file(
    payload: impl IntoLogData,
    topic: &str,
    mut writer: impl Write + Seek,
) -> io::Result<WriteLogData> {
    let Ok(incoming_data) = payload.into_log_data() else {
        return Err(io::Error::new(ErrorKind::InvalidData, "Could not convert"));
    };

    let data = WriteLogData {
        log: incoming_data,
        timestamp: Utc::now(),
        topic: topic.into(),
    };

    writer.seek(SeekFrom::End(0))?;
    data.serialize(&mut writer)?;
    writeln!(writer)?;

    Ok(data)
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, Cursor, Seek, SeekFrom};

    use crate::{LogData, WriteLogData, shared_data::DeviceType, write_to_file};

    #[test]
    fn write_to_log() {
        let mut buff = Cursor::new(Vec::with_capacity(1024));
        let log_data_img_flow = LogData {
            device: DeviceType::IOsApp,
            tag: "ImageFlow".into(),
            data: "Detected 10 cars".into(),
        };
        let log_img_flow = write_to_file(log_data_img_flow, "test/data", &mut buff).unwrap();

        let log_data_vibrate = LogData {
            device: DeviceType::LillyGo,
            tag: "Vibrate".into(),
            data: "brimmm".into(),
        };
        let log_vibrate = write_to_file(log_data_vibrate, "test/data", &mut buff).unwrap();

        buff.seek(SeekFrom::Start(0)).unwrap();

        let mut line = String::new();
        let _ = buff.read_line(&mut line);
        let mut line_bytes = line.as_bytes();
        let deserilized_data_img_flow = WriteLogData::deserialize(&mut line_bytes).unwrap();
        assert_eq!(deserilized_data_img_flow, log_img_flow);

        line = String::new();
        let _ = buff.read_line(&mut line);
        line_bytes = line.as_bytes();
        let deserilized_data_vibrate = WriteLogData::deserialize(&mut line_bytes).unwrap();
        assert_eq!(deserilized_data_vibrate, log_vibrate);
    }
}
