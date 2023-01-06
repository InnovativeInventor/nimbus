use crate::fuse::INode;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Hash)]
pub enum LockStatus {
    HasLock,
    NoLock,
}

type ProjectID = INode; // for now

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Index {
    counter: u64,
    ino_project_lookup: HashMap<INode, ProjectID>, // could use a union-find structure here
    project_lock_status: HashMap<ProjectID, LockStatus>,
    index_lock_status: LockStatus,
}
