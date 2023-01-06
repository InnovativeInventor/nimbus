use log::{info, trace};
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use structopt::StructOpt;

use fuser::{MountOption, Session};

use nimbus::config::read_config;
use nimbus::files::NimbusFS;

#[derive(StructOpt, Debug)]
#[structopt(name = "nimbus")]
struct Opt {
    #[structopt(short, long)]
    mount_directory: PathBuf,

    #[structopt(short, long)]
    local_storage: PathBuf,

    #[structopt(short, long)]
    config: PathBuf,
}

fn main() {
    env_logger::init();
    let args = Opt::from_args();

    info!("Args parsed");

    let config = read_config(args.config);

    info!("{:?}", config);

    let nimbus = NimbusFS::default(args.local_storage, args.mount_directory.clone());

    let session = Session::new(
        nimbus,
        &args.mount_directory,
        &[
            MountOption::DefaultPermissions,
            MountOption::DirSync,
            MountOption::Sync,
        ], // MountOption::AutoUnmount,
    )
    .expect("Could not create session");

    let interrupt = Arc::new(Barrier::new(2));

    let c = Arc::clone(&interrupt);
    ctrlc::set_handler(move || {
        c.wait();
        trace!("Ctrl-C recieved, forwarding to main thread!");
    })
    .expect("Error setting Ctrl-C handler");

    let bg = session.spawn().expect("Session failed to spawn");

    interrupt.wait();
    info!("Ctrl-C recieved, gracefully exiting!");
    bg.join();
    info!("Cleanup successful, exit complete!");
}
