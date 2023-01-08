// use tests::NimbusTester

// fn setup() -> (PathBuf, PathBuf) {
//     let local_storage: TempDir = tempfile::tempdir().unwrap();
//     let mount_directory: TempDir = tempfile::tempdir().unwrap();
// }

// fn destroy(s: PathBuf, m: PathBuf) {
//     fs::remove_dir_all(m);
//     fs::remove_dir_all(s);
// }

// #[test]
// fn test_mount_umount() {
//     let (store_p, mount_p) = setup();
//     let nimbus = NimbusFS::default(store_p.clone(), mount_p.clone());

//     bg.join();
//     destroy(store_p, mount_p);
// }
