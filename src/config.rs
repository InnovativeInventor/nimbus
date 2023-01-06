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
    name: String,
    mode: MachineMode,
    port: usize,
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub struct NetworkMachineConfig {
    command: String,
    endpoint: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    machine: MachineConfig,
    network: HashMap<String, NetworkMachineConfig>,
}

pub fn read_config(config_path: PathBuf) -> Config {
    toml::from_str(&std::fs::read_to_string(config_path).expect("Unable to read config"))
        .expect("Unable to parse config")
}
