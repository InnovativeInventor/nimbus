// use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum MachineMode {
    DevelopmentMode,
    BackupMode,
}

// #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct MachineConfig {
    name: Option<String>,
    command: String,
    mode: MachineMode,
}

// #[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct Config {
    name: String,
    machines: HashMap<String, String>,
}
