use std::fs;
// use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

use libc::{c_int, ENOSYS, EPERM};
use std::path::{Path, PathBuf};

use chrono::prelude::*;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::{Filesystem, KernelConfig, ReplyAttr, ReplyDirectory, Request};
use log::{debug, error, info, trace, warn};

use crate::convert::{convert_file_type, convert_metadata};

pub struct NimbusFS {
    /// This where we store the nimbus files on disk
    /// Not intended to be exposed to users
    pub local_storage: PathBuf,

    /// The last time nimbus was updated
    pub last_updated_utc: DateTime<Utc>,
    pub last_updated_local: SystemTime,

    /// Attribute cache duration
    pub attr_ttl: Duration,
}

impl NimbusFS {
    pub fn default(local_storage: PathBuf) -> NimbusFS {
        // todo: change last_updated to actually be last_updated
        let last_updated = Utc::now();
        NimbusFS {
            local_storage: local_storage,
            last_updated_utc: last_updated,
            last_updated_local: SystemTime::from(last_updated),
            attr_ttl: Duration::new(1, 0), // default to one sec
        }
    }
}

impl Filesystem for NimbusFS {
    fn init(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<(), c_int> {
        info!("Filesystem mounted");
        Ok(())
    }

    fn getattr(&mut self, j_req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if ino == 1 {
            info!("Called (root): getattr(ino: {:#x?})", ino);
            let metadata = fs::metadata(self.local_storage.clone()).unwrap();
            debug!("{:?}", metadata);

            debug!("{:o}", metadata.permissions().mode());
            let attr = convert_metadata(ino, metadata);
            reply.attr(&self.attr_ttl, &attr);
            debug!("{:?}", attr.clone());
        } else {
            info!("Called: getattr(ino: {:#x?})", ino);
            reply.error(ENOSYS);
        }
    }

    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino == 1 {
            info!(
                "Called (root): readdir(ino: {:#x?}, fh: {}, offset: {})",
                ino, fh, offset
            );
            if let Ok(entries) = fs::read_dir(self.local_storage.clone()) {
                for (counter, entry) in entries
                    .skip(offset.try_into().expect("Overflow"))
                    .enumerate()
                {
                    if let Ok(good_entry) = entry {
                        if let Ok(good_file_type) = good_entry.file_type() {
                            if let Ok(good_metadata) = good_entry.metadata() {
                                let result = reply.add(
                                    good_metadata.ino(),
                                    counter as i64 + 1,
                                    convert_file_type(good_file_type),
                                    good_entry.file_name(),
                                );
                                if result == true {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            debug!("{:?}", reply);
            reply.ok();
        } else {
            warn!(
                "[Not Implemented] readdir(ino: {:#x?}, fh: {}, offset: {})",
                ino, fh, offset
            );
            reply.error(ENOSYS);
        }
    }
}
