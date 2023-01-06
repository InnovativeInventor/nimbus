use log::{info, trace};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::{Arc, Barrier};
use structopt::StructOpt;

use fuser::{BackgroundSession, MountOption, Session};

use nimbus::config::read_config;
use nimbus::files::NimbusFS;
use nimbus::server;

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

#[tokio::main]
async fn main() {
    env_logger::init();
    let args = Opt::from_args();
    info!("Args parsed");

    let config = read_config(args.config);
    info!("{:?}", config);

    let nimbus = NimbusFS::default(args.local_storage, args.mount_directory.clone());

    // Listen for interrupt
    let interrupt = Arc::new(Barrier::new(2));
    let c = Arc::clone(&interrupt);
    ctrlc::set_handler(move || {
        c.wait();
        trace!("Ctrl-C recieved, forwarding to main thread!");
    })
    .expect("Error setting Ctrl-C handler");

    // Setup server
    let server = server::build(nimbus.index(), config.machine.endpoint.clone());

    // Setup fuse session
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

    // Spawn stuff
    tokio::spawn(async move { server.await });
    let bg = session.spawn().expect("Session failed to spawn");
    cleanup_mount(interrupt, bg).await;
}

async fn cleanup_mount(interrupt: Arc<Barrier>, bg: BackgroundSession) {
    interrupt.wait();
    info!("Ctrl-C recieved, gracefully exiting!");
    bg.join();
    info!("Cleanup successful, exit complete!");
}
