use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, strum::EnumString)]
pub(crate) enum DeviceType {
    IOsApp,
    LillyGo,
    GalaxyWatch,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct LogData {
    pub(crate) device: DeviceType,
    pub(crate) tag: Box<str>,
    pub(crate) data: Box<str>,
}
