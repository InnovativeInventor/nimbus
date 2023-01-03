use std::fs;
use std::io;
use std::ops::Add;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

use chrono::prelude::*;
use std::time::{Duration, UNIX_EPOCH};

use fuser::{FileAttr, ReplyAttr, ReplyDirectory};
use log::{debug, error, info, trace, warn};

pub fn convert_file_type(file_type: std::fs::FileType) -> fuser::FileType {
    if file_type.is_file() {
        fuser::FileType::RegularFile
    } else if file_type.is_dir() {
        fuser::FileType::Directory
    } else if file_type.is_symlink() {
        fuser::FileType::Symlink
    } else if file_type.is_block_device() {
        fuser::FileType::BlockDevice
    } else if file_type.is_char_device() {
        fuser::FileType::CharDevice
    } else if file_type.is_fifo() {
        fuser::FileType::NamedPipe
    } else if file_type.is_socket() {
        fuser::FileType::Socket
    } else {
        unimplemented!();
    }
}

pub fn convert_metadata(ino: u64, metadata: fs::Metadata) -> FileAttr {
    FileAttr {
        ino: ino,
        size: metadata.len(),
        blocks: metadata.blocks(),
        atime: metadata.accessed().unwrap(),
        mtime: metadata.modified().unwrap(),
        ctime: UNIX_EPOCH.add(Duration::new(metadata.ctime() as u64, 0)),
        crtime: metadata.created().unwrap(), // unsupported (for macOS)
        kind: convert_file_type(metadata.file_type()),
        perm: metadata.permissions().mode().try_into().expect("Overflow"),
        nlink: metadata.nlink().try_into().expect("Overflow"),
        uid: metadata.uid(),
        gid: metadata.gid(),
        rdev: metadata.rdev().try_into().expect("Overflow"),
        blksize: metadata.blksize().try_into().expect("Overflow"),
        flags: 0, // unsupported so far (for macOS)
    }
}
