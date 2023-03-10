use nix::fcntl::renameat2;
use nix::unistd::chown;
use procfs::process::Process;
use procfs::ProcError;
use procfs::ProcError::*;
use rustc_hash::FxHashMap;
use std::ffi::OsStr;
use std::fs;
use std::fs::{File, FileTimes, OpenOptions};
use std::io::{Error, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use libc::{c_int, ENOSYS};
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
use crate::fuse::{parse_error_cint, FileCreate, Fuse, IDirHandle, IFileHandle, INode};
use crate::index::{CanonicalProjectName, Index};

const ROOT_DIR: INode = (1 as u64).into();
const ATTR_TTL: Duration = Duration::new(1, 0);
const PID_POLLING_INTERVAL: Duration = Duration::new(1, 0); // maybe too long?
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

    /// Index locks
    index: Arc<Mutex<Index>>, // maybe use channels
    /// Reference counting for the project locks (do we need atomic?)
    index_refs: FxHashMap<CanonicalProjectName, Arc<AtomicU64>>,

    /// Keep track of file handlers
    ino_open_file_handlers: FxHashMap<INode, Vec<IFileHandle>>,
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
            index_refs: FxHashMap::default(),
            ino_open_file_handlers: FxHashMap::default(),
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

    pub fn canonicize_project_name(&self, path: &PathBuf) -> CanonicalProjectName {
        path.clone()
            .strip_prefix(self.local_storage.clone())
            .expect("Unable to canonicize project name (strip_prefix)")
            .components()
            .find_map(|c| match c {
                std::path::Component::Normal(project) => Some(project),
                _ => None,
            })
            .expect("Unable to canonicize project name (find_map)")
            .into()
    }

    // Increment immediately, then decrement when/if cwd is outside of the project dir
    pub fn pid_cwd_project_ref(&mut self, project: CanonicalProjectName, pid: u32) {
        let counter = self.inc_project_ref(project.clone());
        let mut full_cwd = self.mount_directory.clone();
        full_cwd.push(project);
        let index = self.index();
        std::thread::spawn(move || {
            // check cwd of pid
            // we sleep twice the polling interval to make sure the kernel has time to update procfs (hacky!)
            std::thread::sleep(PID_POLLING_INTERVAL);
            match Process::new(pid as i32) {
                Ok(process) => loop {
                    std::thread::sleep(PID_POLLING_INTERVAL);
                    match process.cwd() {
                        Ok(cwd) => {
                            if !cwd.starts_with(&full_cwd) {
                                info!("left directory {:?}", full_cwd);
                                break;
                            }
                        }
                        Err(PermissionDenied(path)) => {
                            error!(
                                "permission denied to snoop on process {} for project {:?} at path {:?}",
                                pid, full_cwd, path
                                );
                            break;
                        }
                        Err(NotFound(path)) => {
                            info!("process not found at {:?}", path);
                            break;
                        }
                        Err(Incomplete(path)) => {
                            error!("proc file at {:?} has incomplete contents", path)
                            // retry, todo: add retry counter so this function terminates upon repeated Incompletes
                        }
                        Err(Io(err, path)) => {
                            error!("io error {:?} at {:?}", err, path);
                            break;
                        }
                        Err(InternalError(err)) => {
                            error!("internal error {:?}", err);
                            break;
                        }
                        Err(Other(err)) => {
                            error!("other error {:?}", err);
                            break;
                        }
                    }
                },
                Err(PermissionDenied(path)) => error!(
                    "permission denied to snoop on process {} for project {:?} at path {:?}",
                    pid, full_cwd, path
                ),
                Err(NotFound(path)) => info!("process not found at {:?}", path),
                Err(Incomplete(path)) => error!("proc file at {:?} has incomplete contents", path),
                Err(Io(err, path)) => error!("io error {:?} at {:?}", err, path),
                Err(InternalError(err)) => error!("internal error {:?}", err),
                Err(Other(err)) => error!("other error {:?}", err),
            };

            // decrement
            let prev = counter.fetch_sub(1, Ordering::SeqCst);
            info!(
                "project counter for {:?} was at {}, now at {}",
                full_cwd,
                prev,
                prev - 1
            );
            if prev == 0 {
                panic!("reference counting decrement failed/overflowed!");
            } else if prev == 1 {
                // acquire index lock, then check again to be safe
                info!("should release project lock for {:?} now", full_cwd);
                let acquired_index = index.lock().unwrap();
                todo!();
            }
        });
    }

    pub fn inc_project_ref(&mut self, project: CanonicalProjectName) -> Arc<AtomicU64> {
        match self.index_refs.get_mut(&project) {
            Some(inc) => {
                let prev = inc.fetch_add(1, Ordering::SeqCst);
                info!(
                    "project counter for {:?} was at {}, now at {}",
                    project,
                    prev,
                    prev + 1
                );
                if prev == 0 {
                    info!("should obtain project lock now")
                    // acquire index lock, then check again to be safe
                }
                Arc::clone(inc)
                // else if prev == MAX {
                //     panic!("reference counting increment failed/overflowed!");
                // }
            }
            None => {
                let inc = Arc::new(AtomicU64::new(1));
                if self.index_refs.insert(project, Arc::clone(&inc)).is_some() {
                    panic!("should not happen");
                };
                inc
            }
        }
    }

    pub fn dec_project_ref(&mut self, project: CanonicalProjectName) -> Arc<AtomicU64> {
        match self.index_refs.get_mut(&project) {
            Some(dec) => {
                let prev = dec.fetch_sub(1, Ordering::SeqCst);
                info!(
                    "project counter for {:?} was at {}, now at {}",
                    project,
                    prev,
                    prev - 1
                );
                if prev == 0 {
                    panic!("reference counting decrement failed/overflowed!");
                } else if prev == 1 {
                    // acquire project lock first, then check again
                    info!("should release project lock now");
                }
                Arc::clone(dec)
            }
            None => panic!("reference counting decrement failed!"),
        }
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

    pub fn register_file_handler(
        &mut self,
        ino: INode,
        file: std::fs::File,
        use_write_buffer: bool,
    ) -> IFileHandle {
        self.last_file_handle.inc();
        self.file_handlers_map.insert(
            self.last_file_handle.clone(),
            Arc::new(Mutex::new(FileHandler::new(file, 0, use_write_buffer))),
        );
        match self.ino_open_file_handlers.get_mut(&ino) {
            Some(handlers) => handlers.push(self.last_file_handle.clone()),
            None => {
                if self
                    .ino_open_file_handlers
                    .insert(ino, vec![self.last_file_handle.clone()])
                    .is_some()
                {
                    panic!("should not happen")
                }
            }
        }
        self.last_file_handle
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
        ino: INode,
        fh: IFileHandle,
    ) -> std::io::Result<Arc<Mutex<FileHandler>>> {
        match self.ino_open_file_handlers.get_mut(&ino) {
            Some(handlers) => handlers.retain(|x| x != &fh),
            None => {
                panic!("could not close file handler!")
            }
        }

        match self.file_handlers_map.remove(&fh) {
            Some(fh) => Ok(fh),
            None => Err(Error::new(
                ErrorKind::NotFound,
                "file handler deletion failed: file handler not found",
            )),
        }
    }

    pub fn flush_associated_file_handlers(&mut self, ino: INode) -> std::io::Result<()> {
        match self.ino_open_file_handlers.get_mut(&ino) {
            Some(handlers) => {
                for x in handlers.clone() {
                    let fh = self
                        .lookup_file_handler_result(x)
                        .expect("failed to flush file handles");
                    let arc_file_handler = Arc::clone(fh);
                    let mut file_handler = arc_file_handler.lock().unwrap();
                    file_handler.flush()?;
                    file_handler.sync_all()?;
                }
            }
            None => (),
        }
        Ok(())
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
        self.flush_associated_file_handlers(ino)?;
        let mut attr = self.getattr_path(self.lookup_ino_result(&ino)?)?;
        attr.ino = ino.into();
        Ok(attr)
    }

    fn readdir_fs<'a>(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IDirHandle,
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
        req: &Request<'_>,
        parent: INode,
        name: &OsStr,
    ) -> std::io::Result<FileAttr> {
        info!("lookup: lookup called");
        let filename = self.parent_name_lookup_result(parent, name)?;
        info!("lookup: filename {:?}", filename);
        self.pid_cwd_project_ref(self.canonicize_project_name(&filename), req.pid()); // this only really needs to happen on true lookups
        let ino = self.lookup_or_create_path(&filename);

        self.flush_associated_file_handlers(ino)?;
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
        file_handler.offset = file_handler
            .seek(SeekFrom::Start(offset.try_into().expect("Overflow")))?
            .try_into()
            .expect("Overflow");

        // from fuser examples; this is not correct!
        // file_handler.offset = file_handler
        //     .seek(SeekFrom::Start(offset.try_into().expect("Overflow")))?
        //     .try_into()
        //     .expect("Overflow");
        // let mut actual_size = size;
        // let file_size: u32 = file_handler
        //     .metadata()?
        //     .len()
        //     .try_into()
        //     .expect("Over/underflow");
        // if (file_size as i64 > offset) && (file_size as i64 - offset) < size.into() {
        //     actual_size = file_size - offset as u32;
        // }
        // let mut data: Vec<u8> = vec![0; actual_size.try_into().expect("Overflow")];
        // file_handler.read_exact(&mut data)?;

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

        if ino != ROOT_DIR {
            let path = self.lookup_ino_result(&ino)?;
            let project_name = self.canonicize_project_name(path);
            self.inc_project_ref(project_name);
        }

        Ok(self.register_file_handler(ino, fh, use_write_buffer))
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

        if ino != ROOT_DIR {
            let path = self.lookup_ino_result(&ino)?;
            let project_name = self.canonicize_project_name(path);
            self.inc_project_ref(project_name);
        }

        Ok(FileCreate::new(
            attr,
            self.register_file_handler(ino, fh, use_write_buffer),
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
        let times = construct_file_time(atime, mtime, ctime);

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
        let f = self.delete_file_handler_result(ino, fh)?;
        let mut file_handler = f.lock().unwrap();
        file_handler.flush()?; // maybe check bool flag?
        file_handler.sync_all()?;

        if ino != ROOT_DIR {
            let path = self.lookup_ino_result(&ino)?;
            let project_name = self.canonicize_project_name(path);
            self.dec_project_ref(project_name);
        }

        Ok(())
    }
    fn opendir_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        _flags: i32,
    ) -> std::io::Result<IDirHandle> {
        if ino != ROOT_DIR {
            let path = self.lookup_ino_result(&ino)?;
            let project_name = self.canonicize_project_name(path);
            self.inc_project_ref(project_name);
        }
        Ok(0.into())
    }
    fn releasedir_fs(
        &mut self,
        req: &Request<'_>,
        ino: INode,
        fh: IDirHandle,
        flags: i32,
    ) -> std::io::Result<()> {
        if ino != ROOT_DIR {
            let path = self.lookup_ino_result(&ino)?;
            let project_name = self.canonicize_project_name(path);
            self.dec_project_ref(project_name);
        }
        Ok(())
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
        match self.opendir_fs(req, ino.into(), flags) {
            Ok(fh) => reply.opened(fh.into(), 0),
            Err(error) => reply.error(parse_error_cint(error)),
        }
    }

    fn releasedir(&mut self, req: &Request<'_>, ino: u64, fh: u64, flags: i32, reply: ReplyEmpty) {
        match self.releasedir_fs(req, ino.into(), fh.into(), flags) {
            Ok(_) => reply.ok(),
            Err(error) => reply.error(parse_error_cint(error)),
        }
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
    fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {
        info!("forget called!");
    }
}

fn construct_file_time(
    atime: Option<TimeOrNow>,
    mtime: Option<TimeOrNow>,
    ctime: Option<SystemTime>,
) -> FileTimes {
    let now = SystemTime::now();
    let mut times = FileTimes::new();

    if let Some(atime_p) = atime {
        times = match atime_p {
            SpecificTime(t) => times.set_accessed(t),
            Now => times.set_accessed(now),
        };
    }
    if let Some(mtime_p) = mtime {
        times = match mtime_p {
            SpecificTime(t) => times.set_modified(t),
            Now => times.set_modified(now),
        };
    }
    times
}
