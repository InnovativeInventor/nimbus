use crate::fuse::INode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum LockStatus {
    WeHaveLock(String),     // we have the lock
    SomeoneHasLock(String), // somebody has the lock
    NobodyHasLock,          // nobody has the lock
}

pub type CanonicalProjectName = PathBuf; // for now

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Index {
    // counter: u64,
    pub project_lock: HashMap<CanonicalProjectName, LockStatus>, // ProjectID != INode because we may want to rename projects
    pub index_lock: LockStatus,
}

impl Index {
    pub fn new() -> Index {
        Index {
            project_lock: HashMap::new(),
            index_lock: LockStatus::NobodyHasLock,
        }
    }
}
