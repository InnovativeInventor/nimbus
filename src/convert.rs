use std::fs;
use std::fs::{File, OpenOptions};
use std::ops::Add;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

use std::time::{Duration, UNIX_EPOCH};

use libc::{c_int, ENOENT, ENOSYS, EPERM, O_ACCMODE, O_APPEND, O_RDONLY, O_RDWR, O_WRONLY}; // O_EXEC, O_SEARCH,

use fuser::{FileAttr, ReplyAttr, ReplyDirectory};

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

pub fn convert_metadata(metadata: &fs::Metadata) -> FileAttr {
    // todo: better error handling
    FileAttr {
        ino: metadata.ino(),
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

// todo: handle truncate flag
// todo: O_EXEC, O_SEARCH
pub fn parse_flag_options<'a>(flags: i32) -> (OpenOptions, bool) {
    let mut open_options = OpenOptions::new();
    let use_write_buffer = match flags & O_ACCMODE {
        O_RDONLY => {
            open_options.read(true);
            false
        }
        O_WRONLY => {
            open_options.write(true);
            true
        }
        O_RDWR => {
            open_options.read(true).write(true);
            false
        }
        O_APPEND => {
            open_options.append(true);
            false
        }
        other => unimplemented!("Unimplemented flag ({})!", other & O_ACCMODE),
        // O_EXEC => {
        //     unimplemented!("Open with O_EXEC flag is unimplemented!")
        // }
        // O_SEARCH => {
        //     unimplemented!("Open with O_SEARCH flag is unimplemented!")
        // }
    };
    (open_options, use_write_buffer)
}
