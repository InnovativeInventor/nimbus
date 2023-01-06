use log::info;
use std::path::PathBuf;
use structopt::StructOpt;

use fuser::MountOption;

use nimbus::files::NimbusFS;

#[derive(StructOpt, Debug)]
#[structopt(name = "nimbus")]
struct Opt {
    #[structopt(short, long)]
    mount_directory: PathBuf,

    #[structopt(short, long)]
    local_storage: PathBuf,
}

fn main() {
    env_logger::init();
    let args = Opt::from_args();

    info!("Args parsed");
    fuser::mount2(
        NimbusFS::default(args.local_storage, args.mount_directory.clone()),
        args.mount_directory.to_str().unwrap(),
        &[MountOption::AutoUnmount, MountOption::DefaultPermissions],
    )
    .unwrap();
}
