use crate::fuse::INode;
use crate::index::LockStatus::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Mutex};

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

    pub fn acquire_project_lock(
        &mut self,
        project: CanonicalProjectName,
        machine_name: String,
    ) -> bool {
        // returns true on success
        match self.project_lock.get(&project) {
            Some(WeHaveLock(_)) => true, // fast path
            Some(SomeoneHasLock(other)) => {
                // obtain lock
                if other == &machine_name {
                    true
                } else {
                    todo!();
                    // try to request for lock
                    false
                }
            }
            Some(NobodyHasLock) => {
                // obtain lock after inactivity
                // tell everyone that we want the lock
                todo!();
                self.project_lock
                    .insert(project, SomeoneHasLock(machine_name))
                    .expect("this should never happen");
                true
            }
            None => panic!(
                "attempted to acquire lock for {} in {:?}, but does not exist in the index! panic!",
                machine_name, project
            ),
        }
    }
    pub fn release_project_lock(
        &mut self,
        project: CanonicalProjectName,
        machine_name: String,
    ) -> bool {
        // returns true on success
        match self.project_lock.get(&project) {
            Some(WeHaveLock(_)) => {
                todo!();
                true
                // notify others
            }
            Some(SomeoneHasLock(other)) => {
                // obtain lock
                if other == &machine_name {
                    todo!();
                    true // notify others
                } else {
                    false
                }
            }
            Some(NobodyHasLock) => false,
            None => panic!(
                "attempted to acquire lock for {} in {:?}, but does not exist in the index! panic!",
                machine_name, project
            ),
        }
    }
}

pub fn acquire_project_lock(
    index: Arc<Mutex<Index>>,
    project: CanonicalProjectName,
    machine_name: String,
) -> bool {
    let mut index = index.lock().expect("lock failed");
    match index.project_lock.get(&project) {
        Some(WeHaveLock(_)) => true, // fast path
        Some(SomeoneHasLock(other)) => {
            // obtain lock
            if other == &machine_name {
                true
            } else {
                // try to request for lock
                false
            }
        }
        Some(NobodyHasLock) => {
            // obtain lock after inactivity
            // tell everyone that we want the lock
            index
                .project_lock
                .insert(project, SomeoneHasLock(machine_name))
                .expect("this should never happen");
            true
        }
        None => panic!(
            "attempted to acquire lock for {} in {:?}, but does not exist in the index! panic!",
            machine_name, project
        ),
    }
}
