use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub(crate) enum DeviceType {
    IOsApp,
    LillyGo,
    GalaxyWatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LogData<'a> {
    pub(crate) device: DeviceType,
    pub(crate) tag: &'a str,
    pub(crate) data: &'a str,
}
