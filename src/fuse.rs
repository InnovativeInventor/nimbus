use libc::{
    c_int, EISDIR, ENAMETOOLONG, ENOENT, ENOSYS, ENOTEMPTY, EPERM, O_ACCMODE, O_APPEND, O_RDONLY,
    O_RDWR, O_WRONLY, PATH_MAX,
};

use log::{debug, error, info, trace, warn};
use std::ffi::OsStr;
use std::fs::FileType;
use std::io::{ErrorKind, Result}; // O_EXEC, O_SEARCH,
use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

use crate::macros;

use fuser::{
    FileAttr, Filesystem, KernelConfig, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct INode {
    inode: u64,
}

impl const From<u64> for INode {
    fn from(inode: u64) -> Self {
        INode { inode }
    }
}

impl const Into<u64> for INode {
    fn into(self) -> u64 {
        self.inode
    }
}

impl INode {
    pub fn inc(&mut self) {
        self.inode += 1;
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct IFileHandle {
    index: u64,
}
impl const From<u64> for IFileHandle {
    fn from(index: u64) -> Self {
        IFileHandle { index }
    }
}

impl const Into<u64> for IFileHandle {
    fn into(self) -> u64 {
        self.index
    }
}

impl IFileHandle {
    pub fn inc(&mut self) {
        self.index += 1;
    }
}

pub trait Fuse {
    fn duration(&mut self) -> Duration;
    fn init_fs(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<()> {
        info!("Filesystem mounted");
        Ok(())
    }

    fn getattr_fs(&mut self, req: &Request<'_>, ino: INode) -> std::io::Result<FileAttr>;
    fn readdir_fs<'a>(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        offset: i64,
        reply: &'a mut ReplyDirectory,
    ) -> std::io::Result<&'a ReplyDirectory>;
    fn lookup_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
    ) -> std::io::Result<FileAttr>;
    fn read_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> std::io::Result<Vec<u8>>;
    fn write_fs(
        &mut self,
        _req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> std::io::Result<usize>;

    fn open_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        flags: i32,
    ) -> std::io::Result<IFileHandle>; // might also want to return flags in the future

    fn create_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
    ) -> std::io::Result<FileCreate>;

    fn setattr_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        ctime: Option<SystemTime>,
        fh: Option<IFileHandle>,
        crtime: Option<SystemTime>,
        chgtime: Option<SystemTime>,
        bkuptime: Option<SystemTime>,
        flags: Option<u32>,
    ) -> std::io::Result<FileAttr>;

    fn flush_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        lock_owner: u64,
    ) -> std::io::Result<()>;
    fn release_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
    ) -> std::io::Result<()>;

    fn mkdir_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        mode: u32,
        umask: u32,
    ) -> std::io::Result<FileAttr>;
    fn rmdir_fs(&mut self, req: &Request<'_>, parent: INode, name: &OsStr) -> std::io::Result<()>;
    fn rename_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        new_parent: INode,
        new_name: &OsStr,
        flags: u32,
    ) -> std::io::Result<()>;
    fn symlink_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        link: &Path,
    ) -> std::io::Result<FileAttr>;
    fn unlink_fs(&mut self, req: &Request<'_>, parent: INode, name: &OsStr) -> std::io::Result<()>;
    fn readlink_fs(&mut self, req: &Request<'_>, ino: INode)
        -> std::io::Result<std::path::PathBuf>;
}

// todo: create type alias for file handler

pub struct FileCreate {
    pub attr: FileAttr,
    pub fh: IFileHandle,
}

impl FileCreate {
    pub fn new(attr: FileAttr, fh: IFileHandle) -> FileCreate {
        FileCreate { attr: attr, fh: fh }
    }
}

// struct DirectoryAttr {
//     ino: INode,
//     offset: i64,
//     kind: FileType,
//     name: OsStr,
// }

pub fn parse_error_cint(error: std::io::Error) -> c_int {
    info!("parse error: {:?}", error);
    // info!("{}", std::backtrace::Backtrace::capture());
    // panic!();
    match error.kind() {
        ErrorKind::NotFound => ENOENT,
        ErrorKind::InvalidFilename => ENAMETOOLONG, // is this right?
        ErrorKind::DirectoryNotEmpty => ENOTEMPTY,
        _ => todo!(),
    }
}
