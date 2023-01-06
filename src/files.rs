use nix::fcntl::renameat2;
use nix::unistd::chown;
use rustc_hash::FxHashMap;
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
use crate::file_handler::FileHandler;
use crate::fuse::{parse_error_cint, FileCreate, Fuse, IFileHandle, INode};
use crate::index::{CanonicalProjectName, Index};

const ROOT_DIR: INode = (1 as u64).into();
const ATTR_TTL: Duration = Duration::new(1, 0);
// const TIMEOUT: Duration = Duration::new(1, 0);
// const SLEEP_INTERVAL: Duration = Duration::new(0, 10);

pub struct NimbusFS {
    /// This where we store the nimbus files on disk
    /// Not intended to be exposed to users
    local_storage: PathBuf,

    /// Not really needed (useful for rewriting?)
    mount_directory: PathBuf,

    /// The last time nimbus was updated
    last_updated_utc: DateTime<Utc>,
    last_updated_local: SystemTime,

    /// Attribute cache duration
    // pub attr_ttl: Duration,
    generation: u64,

    /// Map containing inode-pathbuf mappings
    ino_file_map: FxHashMap<INode, PathBuf>,
    file_ino_map: FxHashMap<PathBuf, INode>,
    index: Arc<Mutex<Index>>,

    /// Keep track of file handlers
    file_handlers_map: FxHashMap<IFileHandle, Arc<Mutex<FileHandler>>>,
    /// An incrementing counter so we can generate unique file handle ids
    last_file_handle: IFileHandle,
    // Last inode allocated
    last_ino_alloc: INode,
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
            ino_file_map: FxHashMap::default(),
            file_ino_map: FxHashMap::default(),
            index: Arc::new(Mutex::new(Index::new())),
            file_handlers_map: FxHashMap::default(),
            last_file_handle: 0.into(),
            last_ino_alloc: ROOT_DIR,
        };
        nimbus.register_ino(
            ROOT_DIR,
            fs::canonicalize(local_storage).expect("Unable to canonicalize link"),
        );
        nimbus
    }

    pub fn index(&self) -> Arc<Mutex<Index>> {
        Arc::clone(&self.index)
    }

    pub fn canonicize_project_name(&self, path: PathBuf) -> CanonicalProjectName {
        path.strip_prefix(self.local_storage.clone())
            .expect("Unable to canonicize project name (strip_prefix)")
            .components()
            .find_map(|c| match c {
                std::path::Component::Normal(project) => Some(project),
                _ => None,
            })
            .expect("Unable to canonicize project name (find_map)")
            .into()
    }

    // pub fn get_path(&self, path)

    pub fn register_ino(&mut self, ino: INode, path: PathBuf) {
        self.ino_file_map.insert(ino, path.clone());
        self.file_ino_map.insert(path, ino);
    }

    pub fn fresh_ino(&mut self) -> INode {
        self.last_ino_alloc.inc();
        self.last_ino_alloc
    }

    pub fn register_file_handle(
        &mut self,
        file: std::fs::File,
        use_write_buffer: bool,
    ) -> IFileHandle {
        self.last_file_handle.inc();
        self.file_handlers_map.insert(
            self.last_file_handle,
            Arc::new(Mutex::new(FileHandler::new(file, 0, use_write_buffer))),
        );
        self.last_file_handle
    }

    pub fn parent_name_lookup_result(
        &self,
        parent: INode,
        name: &OsStr,
    ) -> std::io::Result<PathBuf> {
        let parent_file = self.lookup_ino_result(&parent)?;
        let mut file = parent_file.clone();
        file.push(name);
        Ok(file)
    }

    pub fn lookup_ino_result(&self, ino: &INode) -> std::io::Result<&PathBuf> {
        match self.ino_file_map.get(ino) {
            Some(path) => Ok(path),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "ino lookup failed: ino not found",
            )),
        }
    }

    // todo: rename to lookup_path
    pub fn lookup_file_result(&self, path: &PathBuf) -> std::io::Result<&INode> {
        match self.file_ino_map.get(path) {
            Some(ino) => Ok(ino),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "file lookup failed: file not found",
            )),
        }
    }

    pub fn lookup_or_create_path(&mut self, path: &PathBuf) -> INode {
        let result = self.file_ino_map.get(path);
        match result {
            Some(ino) => *ino,
            None => {
                let ino = self.fresh_ino();
                self.register_ino(ino, path.clone());
                ino
            }
        }
    }

    pub fn rename_ino(&mut self, old_path: &PathBuf, new_path: &PathBuf) -> std::io::Result<()> {
        let ino = *self.lookup_file_result(&old_path.clone())?;
        self.remove_path(&old_path.clone())?;
        self.register_ino(ino, new_path.clone());
        Ok(())
    }

    pub fn remove_path(&mut self, path: &PathBuf) -> std::io::Result<()> {
        match self.file_ino_map.remove(path) {
            Some(ino) => match self.ino_file_map.remove(&ino) {
                Some(_) => Ok(()),
                None => Err(Error::new(
                    ErrorKind::NotFound,
                    "path remove failed: ino not found",
                )),
            },
            None => Err(Error::new(
                ErrorKind::NotFound,
                "path remove failed: file not found",
            )),
        }
    }

    pub fn lookup_file_handler_result(
        &mut self,
        fh: IFileHandle,
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
        fh: IFileHandle,
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

    fn getattr_fs(&mut self, req: &Request<'_>, ino: INode) -> std::io::Result<FileAttr> {
        let mut attr = self.getattr_path(self.lookup_ino_result(&ino)?)?;
        attr.ino = ino.into();
        Ok(attr)
    }

    fn readdir_fs<'a>(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        offset: i64,
        reply: &'a mut ReplyDirectory,
    ) -> std::io::Result<&'a ReplyDirectory> {
        let entries = fs::read_dir(self.lookup_ino_result(&ino)?)?;
        for (counter, entry) in entries
            .skip(offset.try_into().expect("Overflow")) // convert to result
            .enumerate()
        {
            let good_entry = entry?;
            let file_type = good_entry.file_type()?;
            let ino = self.lookup_or_create_path(&good_entry.path());
            let result = reply.add(
                ino.into(),
                offset + counter as i64 + 1,
                convert_file_type(file_type),
                good_entry.file_name(),
            );
            if result {
                break;
            }
        }
        Ok(reply)
    }

    fn lookup_fs(
        &mut self,
        _req: &Request<'_>,
        parent: INode,
        name: &OsStr,
    ) -> std::io::Result<FileAttr> {
        info!("lookup: lookup called");
        let filename = self.parent_name_lookup_result(parent, name)?;
        info!("lookup: filename {:?}", filename);
        let ino = self.lookup_or_create_path(&filename);
        let mut attr = self.getattr_path(&filename)?;
        attr.ino = ino.into();
        info!("lookup: attr {:?}", attr);
        Ok(attr)
    }

    fn read_fs(
        &mut self,
        _req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
    ) -> std::io::Result<Vec<u8>> {
        let f = self.lookup_file_handler_result(fh)?;
        let arc_file_handler = Arc::clone(f);
        let mut file_handler = arc_file_handler.lock().unwrap();

        // Seek to position
        // let file_handler_offset = file_handler.offset;
        // file_handler.offset = file_handler // corrupt
        //     .file
        //     .seek(SeekFrom::Current(
        //         (offset - file_handler_offset).try_into().expect("Overflow"),
        //     ))?
        //     .try_into()
        //     .expect("Overflow");
        file_handler.offset = file_handler
            .seek(SeekFrom::Start(offset.try_into().expect("Overflow")))?
            .try_into()
            .expect("Overflow");

        // Read
        let mut data: Vec<u8> = vec![0; size.try_into().expect("Overflow")];
        file_handler.read(&mut data)?;
        Ok(data)
    }
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
    ) -> std::io::Result<usize> {
        let f = self.lookup_file_handler_result(fh)?;
        let arc_file_handler = Arc::clone(f);
        let mut file_handler = arc_file_handler.lock().unwrap();

        // Seek to position
        // let file_handler_offset = file_handler.offset;
        // file_handler.offset = file_handler // corrupt
        //     .file
        //     .seek(SeekFrom::Current(
        //         (offset - file_handler_offset).try_into().expect("Overflow"),
        //     ))?
        //     .try_into()
        //     .expect("Overflow");
        file_handler.offset =
            file_handler // corrupt
                .seek(SeekFrom::Start(offset.try_into().expect("Overflow")))?
                .try_into()
                .expect("Overflow");

        // Write
        file_handler.write(data)
    }
    fn open_fs(
        &mut self,
        _req: &Request<'_>,
        ino: INode,
        flags: i32,
    ) -> std::io::Result<IFileHandle> // might also want to return flags in the future
    {
        let (options, use_write_buffer) = parse_flag_options(flags);
        let fh = options.open(self.lookup_ino_result(&ino)?)?;
        Ok(self.register_file_handle(fh, use_write_buffer))
    }
    fn create_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
    ) -> std::io::Result<FileCreate> {
        let filename = self.parent_name_lookup_result(parent, name)?;
        let fh = File::create_new(filename.clone())?;
        let mut attr = self.getattr_path(&filename)?;
        let ino = self.lookup_or_create_path(&filename);
        let (_, use_write_buffer) = parse_flag_options(flags);
        attr.ino = ino.into();
        Ok(FileCreate::new(
            attr,
            self.register_file_handle(fh, use_write_buffer),
        ))
    }
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
    ) -> std::io::Result<FileAttr> {
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
        let filename = self.lookup_ino_result(&ino)?;
        let file = File::options().write(true).open(filename)?;

        file.set_times(times)?;
        if let Some(mode_st) = mode {
            let mut perms = file.metadata()?.permissions();
            perms.set_mode(mode_st);
        }
        if let Some(len) = size {
            file.set_len(len)?;
        }
        if uid.is_some() || gid.is_some() {
            chown(filename, uid.map(|x| x.into()), gid.map(|x| x.into()))?;
        }

        self.getattr_fs(req, ino)
    }

    fn flush_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        lock_owner: u64,
    ) -> std::io::Result<()> {
        let f = self.lookup_file_handler_result(fh)?;
        let arc_file_handler = Arc::clone(f);
        let mut file_handler = arc_file_handler.lock().unwrap();
        file_handler.flush()?;
        file_handler.sync_all()
    }

    fn release_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IFileHandle,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
    ) -> std::io::Result<()> {
        let f = self.delete_file_handler_result(fh)?;
        let mut file_handler = f.lock().unwrap();
        file_handler.flush()?; // maybe check bool flag?
        file_handler.sync_all()
    }
    fn mkdir_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        mode: u32,
        umask: u32,
    ) -> std::io::Result<FileAttr> {
        let dir_path = self.parent_name_lookup_result(parent, name)?;
        fs::create_dir(dir_path.clone())?;
        fs::symlink_metadata(dir_path)?.permissions().set_mode(mode);
        self.lookup_fs(req, parent, name)
    }

    fn rmdir_fs(&mut self, req: &Request<'_>, parent: INode, name: &OsStr) -> std::io::Result<()> {
        let dir_path = self.parent_name_lookup_result(parent, name)?;
        info!(
            "rmdir: there are {:?} files in the dir",
            fs::read_dir(dir_path.clone())?.count()
        );
        fs::remove_dir(dir_path.clone())?;
        info!(
            "removed! parent: {:?}, name: {:?}, path: {:?}",
            parent, name, dir_path
        );
        // self.remove_path(&dir_path)?;
        Ok(())
    }
    fn rename_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        new_parent: INode,
        new_name: &OsStr,
        flags: u32,
    ) -> std::io::Result<()> {
        // todo: check flags for RENAME_EXCHANGE and RENAME_NOREPLACE
        let dir_path = self.parent_name_lookup_result(parent, name)?;
        // let ino = *self.lookup_file_result(&dir_path)?;
        let new_dir_path = self.parent_name_lookup_result(new_parent, new_name)?;
        renameat2(
            None,
            &dir_path,
            None,
            &new_dir_path,
            nix::fcntl::RenameFlags::from_bits_truncate(flags),
        )?;
        // fs::rename(dir_path.clone(), new_dir_path)?;
        self.rename_ino(&dir_path, &new_dir_path)?;
        // self.remove_path(&dir_path)?;
        // let new_ino = self.fresh_ino();
        // self.register_ino(new_ino, dir_path);
        Ok(())
    }
    fn symlink_fs(
        &mut self,
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
        link: &Path,
    ) -> std::io::Result<FileAttr> {
        let sym_path = self.parent_name_lookup_result(parent, name)?;
        std::os::unix::fs::symlink(link, sym_path)?;
        self.lookup_fs(req, parent, name)
    }
    fn unlink_fs(&mut self, req: &Request<'_>, parent: INode, name: &OsStr) -> std::io::Result<()> {
        info!("unlink called");
        let file_path = self.parent_name_lookup_result(parent, name)?;
        fs::remove_file(file_path.clone())?;
        // self.remove_path(&file_path)?;
        Ok(())
    }
    fn readlink_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
    ) -> std::io::Result<std::path::PathBuf> {
        let file = self.lookup_ino_result(&ino)?;
        fs::read_link(file)
    }
}

// This mostly does error handling
impl Filesystem for NimbusFS {
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
        match self.getattr_fs(req, ino.into()) {
            Ok(attr) => reply.attr(&self.duration(), &attr),
            Err(error) => reply.error(parse_error_cint(error)),
        };
    }

    fn readdir(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        match self.readdir_fs(req, ino.into(), fh.into(), offset, &mut reply) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn lookup(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        match self.lookup_fs(req, parent.into(), name) {
            Ok(attr) => {
                reply.entry(&ATTR_TTL, &attr, self.generation);
                info!("reply: {:?}", attr);
            }
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn read(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        match self.read_fs(req, ino.into(), fh.into(), offset, size, flags, lock_owner) {
            Ok(data) => reply.data(&data.into_boxed_slice()),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn write(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        offset: i64,
        data: &[u8],
        write_flags: u32,
        flags: i32,
        lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        match self.write_fs(
            req,
            ino.into(),
            fh.into(),
            offset,
            data,
            write_flags,
            flags,
            lock_owner,
        ) {
            Ok(write_size) => reply.written(write_size.try_into().expect("Overflow")),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn open(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        match self.open_fs(req, ino.into(), flags) {
            Ok(fh) => reply.opened(fh.into(), 0), // todo: check if 0 is the right flag to return here
            Err(error) => reply.error(parse_error_cint(error)),
        };
    }

    fn create(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        flags: i32,
        reply: ReplyCreate,
    ) {
        match self.create_fs(req, parent.into(), name, mode, umask, flags) {
            Ok(file) => reply.created(&ATTR_TTL, &file.attr, self.generation, file.fh.into(), 0), // flags?
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

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
        ctime: Option<SystemTime>,
        fh: Option<u64>,
        crtime: Option<SystemTime>,
        chgtime: Option<SystemTime>,
        bkuptime: Option<SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        match self.setattr_fs(
            req,
            ino.into(),
            mode,
            uid,
            gid,
            size,
            atime,
            mtime,
            ctime,
            fh.map(|x| x.into()),
            crtime,
            chgtime,
            bkuptime,
            flags,
        ) {
            Ok(attr) => reply.attr(&self.duration(), &attr),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn flush(&mut self, req: &Request<'_>, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
        match self.flush_fs(req, ino.into(), fh.into(), lock_owner) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn release(
        &mut self,
        req: &Request<'_>,
        ino: u64,
        fh: u64,
        flags: i32,
        lock_owner: Option<u64>,
        flush: bool,
        reply: ReplyEmpty,
    ) {
        match self.release_fs(req, ino.into(), fh.into(), flags, lock_owner, flush) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn opendir(&mut self, req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        reply.error(ENOSYS);
    }

    fn mkdir(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        reply: ReplyEntry,
    ) {
        match self.mkdir_fs(req, parent.into(), name, mode, umask) {
            Ok(attr) => reply.entry(&ATTR_TTL, &attr, 0),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn rmdir(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        match self.rmdir_fs(req, parent.into(), name) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn rename(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        new_parent: u64,
        new_name: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        match self.rename_fs(req, parent.into(), name, new_parent.into(), new_name, flags) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }
    fn symlink(
        &mut self,
        req: &Request<'_>,
        parent: u64,
        name: &OsStr,
        link: &Path,
        reply: ReplyEntry,
    ) {
        match self.symlink_fs(req, parent.into(), name, link) {
            Ok(attr) => reply.entry(&ATTR_TTL, &attr, 0),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }
    fn unlink(&mut self, req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        match self.unlink_fs(req, parent.into(), name) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }
    fn readlink(&mut self, req: &Request<'_>, ino: u64, reply: ReplyData) {
        match self.readlink_fs(req, ino.into()) {
            Ok(loc) => reply.data(
                loc.to_str()
                    .expect("Unable to convert PathBuf to str")
                    .as_bytes(),
            ),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }
}
