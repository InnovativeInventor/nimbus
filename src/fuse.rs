use libc::{
    c_int, EISDIR, ENAMETOOLONG, ENOENT, ENOSYS, EPERM, O_ACCMODE, O_APPEND, O_RDONLY, O_RDWR,
    O_WRONLY, PATH_MAX,
};

use log::{debug, error, info, trace, warn};
use std::io::Result; // O_EXEC, O_SEARCH,
use std::time::{Duration, Instant, SystemTime};

use fuser::{
    FileAttr, Filesystem, KernelConfig, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};

pub trait Fuse {
    fn duration(&mut self) -> Duration;
    fn init_fs(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<()> {
        info!("Filesystem mounted");
        Ok(())
    }

    fn getattr_fs(&mut self, req: &Request<'_>, ino: u64) -> std::io::Result<FileAttr>;
}

pub fn parse_error_cint(error: std::io::Error) -> c_int {
    todo!();
}

impl Filesystem for dyn Fuse {
    fn init(
        &mut self,
        req: &Request<'_>,
        config: &mut KernelConfig,
    ) -> std::result::Result<(), c_int> {
        match self.init_fs(req, config) {
            Ok(()) => Ok(()),
            Err(error) => Err(parse_error_cint(error)),
        }
    }

    fn getattr(&mut self, req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        match self.getattr_fs(req, ino) {
            Ok(attr) => reply.attr(&self.duration(), &attr),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }
}
