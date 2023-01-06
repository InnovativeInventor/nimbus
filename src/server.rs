use crate::config::{read_config, Config};
use crate::files::NimbusFS;
use crate::index::{Index, LockStatus::*};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Barrier, Mutex};
use warp::{Filter, Server};

pub async fn build(index: Arc<Mutex<Index>>, endpoint: String) {
    // Setup routes
    let nimbus_index = index.clone();
    let acquire_project_lock =
        warp::path!("lock" / "acquire" / String / String).map(move |machine_name: String, project_name: String| {
            let project_path = PathBuf::from_str(&project_name).expect("Could not convert to PathBuf");
            let mut index = nimbus_index.lock().expect("lock failed");
            match index.project_lock.get(&project_path) {
                Some(WeHaveLock(_)) => "fail",                    // mine, not yours
                Some(SomeoneHasLock(other)) => {
                    if other == &machine_name { // you already have the lock
                        "acquired"
                    } else {// someone else has the lock and it ain't you
                        "fail"
                    }
                }
                Some(NobodyHasLock) => {
                    index
                        .project_lock
                        .insert(PathBuf::from_str(&project_name).expect("Could not convert to PathBuf"), SomeoneHasLock(machine_name))
                        .expect("this should never happen");
                    "acquired"
                }
                None => panic!(
                    "attempted to acquire lock for {} in {:?}, but does not exist in the index! panic!", machine_name, project_name
                ),
            }
        });
    let nimbus_index = index.clone();
    let update_and_release_project_lock =
        warp::path!("lock" / "release" / String / String).map(move |machine_name: String, project_name: String| {
            let project_path = PathBuf::from_str(&project_name).expect("Could not convert to PathBuf");
            let mut index = nimbus_index.lock().expect("lock failed");
            match index.project_lock.get(&project_path) {
                Some(WeHaveLock(_)) => "fail", // we have the lock
                Some(SomeoneHasLock(other)) => {
                    if other == &machine_name { // you have the lock, good
                        index
                            .project_lock
                            .insert(project_path, NobodyHasLock)
                            .expect("this should never happen");
                        "released"
                    } else { // someone else has the lock and it ain't you
                        "fail"
                    }
                } // you have the lock, good
                Some(NobodyHasLock) => "fail", // you don't have the lock
                None => panic!(
                    "attempted to release lock for {} in {:?}, but does not exist in the index! panic!", machine_name, project_name
                ),
            }
        });
    let nimbus_index = index.clone();
    let acquire_index_lock =
        warp::path!("index" / "lock" / "acquire" / String).map(move |machine_name: String| {
            let mut index = nimbus_index.lock().expect("lock failed");
            match &index.index_lock {
                WeHaveLock(_) => "fail", // mine, not yours
                SomeoneHasLock(other) => {
                    if other == &machine_name {
                        // you already have the lock
                        "acquired"
                    } else {
                        // someone else has the lock and it ain't you
                        "fail"
                    }
                }
                NobodyHasLock => {
                    index.index_lock = SomeoneHasLock(machine_name);
                    "acquired"
                }
            }
        });
    let nimbus_index = index.clone();
    let update_and_release_index_lock =
        warp::path!("index" / "lock" / "release" / String).map(move |machine_name: String| {
            let mut index = nimbus_index.lock().expect("lock failed");
            match &index.index_lock {
                WeHaveLock(_) => "fail", // mine, not yours
                SomeoneHasLock(other) => {
                    if other == &machine_name {
                        // you already have the lock
                        index.index_lock = NobodyHasLock;
                        "released"
                    } else {
                        // you didn't have the lock
                        "fail"
                    }
                }
                NobodyHasLock => "fail",
            }
        });
    let routes = warp::get().and(
        acquire_project_lock
            .or(update_and_release_project_lock)
            .or(acquire_index_lock)
            .or(update_and_release_index_lock),
    );
    warp::serve(routes)
        .run(SocketAddr::from_str(&endpoint).expect("supplied endpoint failed to parse"))
        .await;
}
