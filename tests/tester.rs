use fuser::{BackgroundSession, MountOption, Session};
use nimbus::files::NimbusFS;
use nimbus::server;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

struct NimbusTester {
    bg: BackgroundSession,
    local_storage: PathBuf,
    mount_directory: PathBuf,
}

impl NimbusTester {
    fn new() -> NimbusTester {
        let local_storage: TempDir = tempfile::tempdir().unwrap();
        let mount_directory: TempDir = tempfile::tempdir().unwrap();

        let (store_p, mount_p) = (local_storage.into_path(), mount_directory.into_path());
        let nimbus = NimbusFS::default(store_p.clone(), mount_p.clone());

        let session = Session::new(
            nimbus,
            &mount_p,
            &[
                MountOption::DefaultPermissions,
                MountOption::DirSync,
                MountOption::Sync,
                MountOption::NoAtime,
            ],
        )
        .expect("Could not create session");
        let mut bg = session.spawn().expect("Session failed to spawn");

        NimbusTester {
            bg: bg,
            local_storage: store_p,
            mount_directory: mount_p,
        }
    }
}

impl Drop for NimbusTester {
    fn drop(&mut self) {
        log::info!("{:?}", self.bg);
        fs::remove_dir_all(self.local_storage.clone());
        fs::remove_dir_all(self.mount_directory.clone());
    }
}
