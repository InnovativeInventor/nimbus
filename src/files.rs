use nix::unistd::chown;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::fs::{File, FileTimes, OpenOptions};
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::{Arc, Mutex};
// use std::thread;

// todo: handle truncate flag
use libc::{
    c_int, EISDIR, ENAMETOOLONG, ENOENT, ENOSYS, EPERM, O_ACCMODE, O_APPEND, O_RDONLY, O_RDWR,
    O_WRONLY, PATH_MAX,
}; // O_EXEC, O_SEARCH,
use std::path::PathBuf;

use chrono::prelude::*;
use std::time::{Duration, Instant, SystemTime};

use fuser::TimeOrNow::{Now, SpecificTime};
use fuser::{
    FileAttr, Filesystem, KernelConfig, Reply, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory,
    ReplyEmpty, ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};
use log::{debug, error, info, trace, warn};

use crate::convert::{convert_file_type, convert_metadata, parse_flag_options};
use crate::fuse::Fuse;

const ROOT_DIR: u64 = 1;
const ATTR_TTL: Duration = Duration::new(1, 0);
const TIMEOUT: Duration = Duration::new(1, 0);
const SLEEP_INTERVAL: Duration = Duration::new(0, 10);

pub struct FileHandler {
    offset: i64, // todo: should always be positive (maybe change type)
    file: std::fs::File,
}

pub struct NimbusFS {
    /// This where we store the nimbus files on disk
    /// Not intended to be exposed to users
    pub local_storage: PathBuf,

    /// Needed for symlink rewriting
    pub mount_directory: PathBuf,

    /// The last time nimbus was updated
    pub last_updated_utc: DateTime<Utc>,
    pub last_updated_local: SystemTime,

    /// Attribute cache duration
    // pub attr_ttl: Duration,
    pub generation: u64,

    /// Map containing inode-pathbuf mappings
    ino_file_map: HashMap<u64, PathBuf>,
    file_ino_map: HashMap<PathBuf, u64>,
    // Last inode allocated
    // last_ino_alloc: u64,
    /// Keep track of file handlers
    file_handlers_map: HashMap<u64, Arc<Mutex<FileHandler>>>,
    /// An incrementing counter so we can generate unique file handle ids
    last_file_handle: u64,
    // last_alloced_ino: u64,
}

impl NimbusFS {
    pub fn default(local_storage: PathBuf, mount_directory: PathBuf) -> NimbusFS {
        // todo: change last_updated to actually be last_updated
        let last_updated = Utc::now();
        let mut nimbus = NimbusFS {
            local_storage: fs::canonicalize(local_storage.clone())
                .expect("Unable to canonicalize link"),
            mount_directory: fs::canonicalize(mount_directory)
                .expect("Unable to canonicalize link"),
            last_updated_utc: last_updated,
            last_updated_local: SystemTime::from(last_updated),
            // attr_ttl: Duration::new(1, 0), // default to one sec
            generation: 0,
            ino_file_map: HashMap::new(),
            file_ino_map: HashMap::new(),
            file_handlers_map: HashMap::new(),
            last_file_handle: 0, // last_ino_alloc: ROOT_DIR,
                                 // last_alloced_ino: ROOT_DIR,
        };
        nimbus.register_ino(
            ROOT_DIR,
            fs::canonicalize(local_storage).expect("Unable to canonicalize link"),
        );
        nimbus
    }

    // pub fn fresh_ino(&mut self) -> u64 {
    //     self.last_alloced_ino += 1;
    //     self.last_alloced_ino
    // }

    pub fn register_ino(&mut self, ino: u64, path: PathBuf) {
        self.ino_file_map.insert(ino, path.clone());
        self.file_ino_map.insert(path, ino);
    }

    pub fn parent_name_lookup(&mut self, parent: u64, name: &OsStr) -> Option<PathBuf> {
        if let Some(parent_file) = self.lookup_ino(&parent) {
            // todo: might have to store all parents at some point
            info!("Hit on parent!");
            let mut file = parent_file.clone();
            file.push(name);
            info!("Child: {:?}", file);
            Some(file)
        } else {
            None
        }
    }

    pub fn lookup_ino(&self, ino: &u64) -> Option<&PathBuf> {
        self.ino_file_map.get(ino)
    }

    // todo: rename to lookup_path
    pub fn lookup_file(&self, path: &PathBuf) -> Option<&u64> {
        self.file_ino_map.get(path)
    }

    pub fn register_file_handle(&mut self, file: std::fs::File) -> u64 {
        self.last_file_handle += 1;
        self.file_handlers_map.insert(
            self.last_file_handle,
            Arc::new(Mutex::new(FileHandler {
                offset: 0,
                file: file,
            })),
        );
        self.last_file_handle
    }

    pub fn lookup_file_handler(&mut self, fh: u64) -> Option<&Arc<Mutex<FileHandler>>> {
        self.file_handlers_map.get(&fh)
    }

    pub fn delete_file_handler(&mut self, fh: u64) -> Option<Arc<Mutex<FileHandler>>> {
        self.file_handlers_map.remove(&fh)
    }

    // Result variants
    pub fn lookup_ino_result(&self, ino: &u64) -> std::io::Result<&PathBuf> {
        match self.ino_file_map.get(ino) {
            Some(path) => Ok(path),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "ino lookup failed: ino not found",
            )),
        }
    }

    // todo: rename to lookup_path
    pub fn lookup_file_result(&self, path: &PathBuf) -> std::io::Result<&u64> {
        match self.file_ino_map.get(path) {
            Some(ino) => Ok(ino),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "file lookup failed: file not found",
            )),
        }
    }

    pub fn lookup_file_handler_result(
        &mut self,
        fh: u64,
    ) -> std::io::Result<&Arc<Mutex<FileHandler>>> {
        match self.file_handlers_map.get(&fh) {
            Some(fh) => Ok(fh),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "file handler lookup failed: file handler not found",
            )),
        }
    }

    pub fn delete_file_handler_result(
        &mut self,
        fh: u64,
    ) -> std::io::Result<Arc<Mutex<FileHandler>>> {
        match self.file_handlers_map.remove(&fh) {
            Some(fh) => Ok(fh),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "file handler deletion failed: file handler not found",
            )),
        }
    }

    pub fn count_file_handlers(&mut self) -> usize {
        self.file_handlers_map.len()
    }

    fn getattr_path(&self, path: &PathBuf) -> Result<FileAttr, std::io::Error> {
        let metadata = fs::symlink_metadata(path)?; // todo: better error handling
        Ok(convert_metadata(&metadata))
    }
}

impl Fuse for NimbusFS {
    fn duration(&mut self) -> Duration {
        ATTR_TTL
    }

    fn getattr_fs(&mut self, req: &Request<'_>, ino: u64) -> std::io::Result<FileAttr> {
        let mut attr = self.getattr_path(self.lookup_ino_result(&ino)?)?;
        attr.ino = ino;
        return Ok(attr);
    }
}

impl Filesystem for NimbusFS {
    fn init(&mut self, _req: &Request<'_>, _config: &mut KernelConfig) -> Result<(), c_int> {
        info!("Filesystem mounted");
        Ok(())
    }

    fn getattr(&mut self, _j_req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        info!("Called: getattr(ino: {:#x?})", ino);
        if let Some(file) = self.lookup_ino(&ino) {
            info!("Hit!");
            match self.getattr_path(&file) {
                Ok(mut attr) => {
                    attr.ino = ino;
                    reply.attr(&ATTR_TTL, &attr);
                }
                Err(error) => match error.kind() {
                    ErrorKind::NotFound => reply.error(ENOENT),
                    _ => crate::unhandled!("Unimplemented error in getattr: {:?}", error),
                },
            }
        } else {
            info!("Miss!");
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
        info!(
            "Called (root): readdir(ino: {:#x?}, fh: {}, offset: {})",
            ino, fh, offset
        );
        if let Some(file) = self.lookup_ino(&ino) {
            info!("Hit!");
            if let Ok(entries) = fs::read_dir(file) {
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
                                self.register_ino(good_metadata.ino(), good_entry.path()); // opportunistically add
                                if result {
                                    break;
                                }
                            }
                        }
                    }
                }
            }
            debug!("Replied with: {:?}", reply);
            reply.ok();
        } else {
            info!("Miss!");
            reply.error(ENOSYS);
        }
    }

    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        info!("lookup(parent: {:#x?}, name {:?})", parent, name);
        if let Some(file) = self.parent_name_lookup(parent, name) {
            match self.getattr_path(&file) {
                Ok(attr) => {
                    self.register_ino(attr.ino, file.clone()); // opportunistically add
                    reply.entry(&ATTR_TTL, &attr, self.generation);
                    info!("Attrs found for {:?}", file);
                    info!("reply: {:?}", attr);
                }
                Err(err) => match err.kind() {
                    ErrorKind::NotFound => {
                        info!("ENOENT returned for {:?}", file);
                        reply.error(ENOENT);
                    }
                    ErrorKind::InvalidFilename => {
                        info!("ENAMETOOLONG returned for {:?}", file);
                        reply.error(ENAMETOOLONG);
                    }
                    err => crate::unhandled!("Other error codes are unimplemented: {:?}", err),
                },
            }
        } else {
            info!("Miss!");
            reply.error(ENOSYS);
        }
    }

    fn read(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        // todo: implement file handling logic
        // todo: maybe implement flag handling logic
        info!(
            "read(ino: {:#x?}, fh: {}, offset: {}, size: {}, \
            flags: {:#x?}, lock_owner: {:?})",
            ino, fh, offset, size, flags, lock_owner
        );
        if let Some(f) = self.lookup_file_handler(fh) {
            let mut data: Vec<u8> = vec![0; size.try_into().expect("Overflow")];
            let arc_file_handler = Arc::clone(f);
            let mut file_handler = arc_file_handler.lock().unwrap();
            let file_handler_offset = file_handler.offset;
            match file_handler.file.seek(SeekFrom::Current(
                (offset - file_handler_offset).try_into().expect("Overflow"),
            )) {
                Ok(offset) => {
                    file_handler.offset = offset as i64;
                }
                Err(_) => {
                    reply.error(ENOSYS);
                    error!(
                        "Unimplemented read() error (seek with {})",
                        file_handler_offset - offset
                    );
                    return;
                }
            }
            match file_handler.file.read(&mut data) {
                Ok(_) => {
                    reply.data(&data.into_boxed_slice());
                }
                Err(_) => {
                    reply.error(ENOSYS);
                    error!("Unimplemented read() error (read)");
                    return;
                }
            }
        } else {
            reply.error(ENOSYS);
            error!("Unimplemented read() error (lookup_file_handler)");
        }
    }

    fn write(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        debug!(
            "write(ino: {:#x?}, fh: {}, offset: {}, data.len(): {}, \
            write_flags: {:#x?}, flags: {:#x?}, lock_owner: {:?})",
            ino,
            fh,
            offset,
            data.len(),
            write_flags,
            flags,
            lock_owner
        );
        if let Some(f) = self.lookup_file_handler(fh) {
            let arc_file_handler = Arc::clone(f);
            let mut file_handler = arc_file_handler.lock().unwrap();
            let file_handler_offset = file_handler.offset;
            match file_handler.file.seek(SeekFrom::Current(
                (offset - file_handler_offset).try_into().expect("Overflow"),
            )) {
                Ok(offset) => {
                    file_handler.offset = offset as i64;
                }
                Err(_) => {
                    reply.error(ENOSYS);
                    error!(
                        "Unimplemented write() error (seek with {})",
                        file_handler_offset - offset
                    );
                    return;
                }
            }
            match file_handler.file.write(&data) {
                Ok(bytes_written) => {
                    reply.written(bytes_written.try_into().expect("Buffer overflow"));
                }
                Err(_) => {
                    reply.error(ENOSYS);
                    error!("Unimplemented write() error (write)");
                    return;
                }
            }
        } else {
            reply.error(ENOSYS);
            error!("Unimplemented read() error (lookup_file_handler)");
        }
    }

    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        info!("Called open(ino: {:?}", ino);
        if let Some(file) = self.lookup_ino(&ino) {
            info!("Hit (open()): {:?}", file);
            let options = parse_flag_options(flags);
            match options.open(file) {
                Ok(fh) => reply.opened(self.register_file_handle(fh), 0), // todo: check if 0 is the right flag to return here
                Err(error) => match error.kind() {
                    ErrorKind::NotFound => reply.error(ENOENT),
                    _ => crate::unhandled!("Unimplemented open() error (open) with {:?}", error),
                },
            };
        }
    }

    fn create(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        info!(
            "create(parent: {:#x?}, name: {:?}, mode: {}, umask: {:#x?}, \
            flags: {:#x?})",
            parent, name, mode, umask, flags
        );

        if let Some(file) = self.parent_name_lookup(parent, name) {
            // let mut options = parse_flag_options(flags);
            match File::create_new(file.clone()) {
                Ok(fh) => {
                    // let now = Instant::now();
                    // while (!file.exists()) && now.elapsed() < TIMEOUT {
                    //     thread::sleep(SLEEP_INTERVAL);
                    // }
                    match self.getattr_path(&file) {
                        Ok(attr) => {
                            info!("File created");
                            self.register_ino(attr.ino, file); // opportunistically add
                            reply.created(
                                &ATTR_TTL,
                                &attr,
                                self.generation,
                                self.register_file_handle(fh),
                                0,
                            );
                        }
                        Err(err) => match err.kind() {
                            _ => crate::unhandled!(
                                "create() error codes need to be implemented, {:?}",
                                err
                            ),
                        },
                    }
                }
                Err(err) => crate::unhandled!(
                    "unhandled create() error {:?} with options {:?}",
                    err,
                    () // options
                ),
            }
        } else {
            reply.error(ENOSYS);
            crate::unhandled!("Unimplemented create() error (lookup)");
        }
    }

    // chained
    fn setattr(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        // todo: truncate option not handled yet
        info!(
            "Called setattr(ino: {:#x?}, mode: {:?}, uid: {:?}, \
             gid: {:?}, size: {:?}, atime: {:?}, mtime: {:?}, fh: {:?}, flags: {:?})",
            ino, mode, uid, gid, size, atime, mtime, fh, flags
        );
        let now = SystemTime::now();
        let mut times = FileTimes::new();
        if let Some(atime_p) = atime {
            times = match atime_p {
                SpecificTime(t) => times.set_accessed(t),
                Now => times.set_accessed(now),
            };
        }
        if let Some(mtime_p) = atime {
            times = match mtime_p {
                SpecificTime(t) => times.set_modified(t),
                Now => times.set_modified(now),
            };
        }

        // Currently, the file handler option is ignored
        // if let Some(file_handler) = fh {
        //     if let Some(f) = self.lookup_file_handler(file_handler) {
        //         let arc_file_handler = Arc::clone(f);
        //         let mut file_handler = arc_file_handler.lock().unwrap();
        //         // let file_handler_offset = file_handler.offset;
        //         file_handler
        //             .file
        //             .set_times(times)
        //             .expect("setattr() failed to set times");
        //     }
        if let Some(filename) = self.lookup_ino(&ino) {
            match File::options().write(true).open(filename) {
                Ok(file) => {
                    file.set_times(times)
                        .expect("setattr() failed to set times");
                    if let Some(mode_st) = mode {
                        let mut perms = file
                            .metadata()
                            .expect("setattr() failed to fetch metadata")
                            .permissions();
                        perms.set_mode(mode_st);
                    }
                    if let Some(len) = size {
                        file.set_len(len).expect("setattr() failed to set size");
                    }
                    // let the file get closed here
                    match chown(filename, uid.map(|x| x.into()), gid.map(|x| x.into())) {
                        Ok(_) => self.getattr(req, ino, reply),
                        Err(error) => {
                            reply.error(ENOSYS);
                            crate::unhandled!(
                                "Unimplemented error handling in setattr(): {:?}",
                                error
                            )
                        }
                    }
                    // todo: set uid, gid, etc.
                }
                Err(error) => match error.kind() {
                    ErrorKind::IsADirectory => {
                        info!("Returned EISDIR for {:?}", filename);
                        reply.error(EISDIR)
                    }
                    error => {
                        reply.error(ENOSYS);
                        crate::unhandled!("Unimplemented error handling in setattr(): {:?}", error)
                    }
                },
            }
        } else {
            crate::unhandled!("Unimplemented error handling in setattr()")
        }
    }

    fn flush(&mut self, _req: &Request<'_>, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
        info!(
            "flush(ino: {:#x?}, fh: {}, lock_owner: {:?})",
            ino, fh, lock_owner
        );
        if let Some(f) = self.lookup_file_handler(fh) {
            let arc_file_handler = Arc::clone(f);
            let mut file_handler = arc_file_handler.lock().unwrap();
            if file_handler.file.flush().is_ok() && file_handler.file.sync_all().is_ok() {
                reply.ok();
            }
        } else {
            reply.error(ENOSYS);
        }
    }

    fn release(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        info!("release(fh: {:?}) called", fh);
        match self.delete_file_handler(fh) {
            Some(fh) => {
                let mut f = fh.lock().expect("Unable to obtain mutex in release()");
                if f.file.flush().is_ok() && f.file.sync_all().is_ok() {
                    reply.ok();
                } else {
                    crate::unhandled!("Unimplemented error handling in release()");
                }
            }
            None => {
                crate::unhandled!("Unimplemented error handling in release()");
            }
        }

        info!(
            "There are {} file handers remaining",
            self.count_file_handlers()
        );
    }

    // chained
    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        info!(
            "mkdir(parent: {:#x?}, name: {:?}, mode: {}, umask: {:#x?})",
            parent, name, mode, umask
        );
        if let Some(dir_path) = self.parent_name_lookup(parent, name) {
            match fs::create_dir(dir_path.clone()) {
                Ok(_) => {
                    let mut perms = fs::symlink_metadata(dir_path)
                        .expect("mkdir() failed to fetch metadata")
                        .permissions();
                    perms.set_mode(mode);
                    self.lookup(req, parent, name, reply);
                }
                Err(_) => {
                    crate::unhandled!("Unimplemeneted error handling in mkdir()");
                }
            }
        } else {
            crate::unhandled!("Unimplemeneted error handling in mkdir()");
        }
    }

    fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        // todo: need to postpone removal and implement inode counter
        info!("rmdir(parent: {:#x?}, name: {:?})", parent, name,);
        if let Some(dir_path) = self.parent_name_lookup(parent, name) {
            match fs::remove_dir(dir_path) {
                Ok(_) => reply.ok(),
                Err(_) => {
                    crate::unhandled!("Unimplemeneted error handling in mkdir()");
                }
            }
        } else {
            crate::unhandled!("Unimplemeneted error handling in rmdir()");
        }
    }

    fn rename(
        &mut self,
        _req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        new_parent: u64,
        new_name: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        info!(
            "rename(parent: {:#x?}, name: {:?}, newparent: {:#x?}, \
            newname: {:?}, flags: {})",
            parent, name, new_parent, new_name, flags,
        );
        if let Some(dir_path) = self.parent_name_lookup(parent, name) {
            if let Some(new_dir_path) = self.parent_name_lookup(new_parent, new_name) {
                match fs::rename(dir_path, new_dir_path) {
                    Ok(_) => reply.ok(),
                    Err(_) => {
                        crate::unhandled!("Unimplemeneted error handling in mkdir()");
                    }
                }
            } else {
                crate::unhandled!("Unimplemeneted error handling in mkdir()");
            }
        } else {
            crate::unhandled!("Unimplemeneted error handling in rmdir()");
        }
    }

    // chained
    fn symlink(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        link: &Path,
        reply: ReplyEntry,
    ) {
        info!(
            "symlink(parent: {:#x?}, name: {:?}, link: {:?})",
            parent, name, link,
        );
        if let Some(sym_path) = self.parent_name_lookup(parent, name) {
            match std::os::unix::fs::symlink(link, sym_path) {
                Ok(_) => {
                    self.lookup(req, parent, name, reply);
                }
                Err(_) => crate::unhandled!("Unimplemented error handling in symlink()"),
            }
        } else {
            crate::unhandled!("Unimplemented error handling in symlink()");
        }
    }

    fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if let Some(file_path) = self.parent_name_lookup(parent, name) {
            match fs::remove_file(file_path) {
                Ok(_) => reply.ok(),
                Err(_) => crate::unhandled!("Unimplemented error handling in symlink()"),
            }
        } else {
            crate::unhandled!("Unimplemented error handling in symlink()");
        }
    }

    fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {
        debug!("Calling readlink(ino: {:#x?})", ino);
        if let Some(file) = self.lookup_ino(&ino) {
            info!("Reading symlink at: {:?}", file);
            match fs::read_link(file) {
                Ok(loc) => {
                    reply.data(
                        loc.to_str()
                            .expect("Unable to convert PathBuf to str")
                            .as_bytes(),
                    );
                }
                Err(_) => crate::unhandled!(),
            }
        } else {
            reply.error(ENOSYS);
        }
    }

    // fn destroy(&mut self) {
    //     info!("Leaving filesystem and unmounting!");
    // }

    // fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
    //     crate::unhandled!("Unimplemented opendir() call");
    //     reply.opened(0, 0);
    // }
    // fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
    //     crate::unhandled!("Unimplemented fstatfs() call");
    //     reply.statfs(0, 0, 0, 0, 0, 512, 255, 0);
    // }
}
