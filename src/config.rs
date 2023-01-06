use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum MachineMode {
    DevelopmentMode,
    BackupMode,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct MachineConfig {
    pub name: String,
    pub mode: MachineMode,
    pub endpoint: String,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct NetworkMachineConfig {
    pub command: String,
    pub endpoint: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub machine: MachineConfig,
    pub network: HashMap<String, NetworkMachineConfig>,
}

pub fn read_config(config_path: PathBuf) -> Config {
    toml::from_str(&std::fs::read_to_string(config_path).expect("Unable to read config"))
        .expect("Unable to parse config")
}
