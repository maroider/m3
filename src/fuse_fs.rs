use std::{
    ffi::{c_int, CStr, OsStr, OsString},
    fs::File,
    io::{self, Read, Seek, SeekFrom},
    iter,
    mem::{self, offset_of},
    os::{
        fd::FromRawFd,
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime},
};

use color_eyre::eyre;
use fuser::{
    FileAttr, FileType, Filesystem, KernelConfig, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, Request,
};
use indexmap::IndexMap;
use libc::{ino64_t, stat64, ENOENT};
use slab::Slab;
use tracing::{debug, debug_span, error, info, trace, warn};

static EXIT: AtomicBool = AtomicBool::new(false);

pub fn launch() -> eyre::Result<()> {
    ctrlc::set_handler(|| EXIT.store(true, Ordering::SeqCst))?;
    let path = "./testdir/gamedir";
    let mount_point = MountPoint::open(path)?;
    let _handle = fuser::spawn_mount2(ModdingFileSystem::new(mount_point), path, &[])?;

    loop {
        if EXIT.load(Ordering::SeqCst) {
            break;
        }
    }

    Ok(())
}

#[derive(Debug)]
struct ModdingFileSystem {
    mount_point: MountPoint,
    open_files: Slab<File>,
}

impl ModdingFileSystem {
    fn new(mount_point: MountPoint) -> Self {
        Self {
            mount_point,
            open_files: Slab::new(),
        }
    }
}

impl Filesystem for ModdingFileSystem {
    #[tracing::instrument(skip_all)]
    fn init(&mut self, _: &Request<'_>, _: &mut KernelConfig) -> Result<(), c_int> {
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    fn destroy(&mut self) {
        info!("Unmounting modding file system");
    }

    #[tracing::instrument(skip(self, _req, parent, reply))]
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if let Some((_, entry)) = self
            .mount_point
            .read_dir(parent)
            .find(|(entry_name, _)| *entry_name == name)
        {
            let stat = &entry.stat;
            info!("{name:?} is ino {}", stat.st_ino);
            reply.entry(
                &Duration::from_secs(1),
                &FileAttr {
                    ino: stat.st_ino,
                    size: stat.st_size as u64,
                    blocks: stat.st_blocks as u64,
                    atime: SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_atime as u64),
                    mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_mtime as u64),
                    ctime: SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_ctime as u64),
                    // macos only, don't care
                    crtime: SystemTime::UNIX_EPOCH,
                    kind: stat_file_type(stat).unwrap(),
                    perm: stat.st_mode as u16,
                    nlink: stat.st_nlink as u32,
                    uid: stat.st_uid,
                    gid: stat.st_gid,
                    rdev: stat.st_rdev as u32,
                    blksize: stat.st_blksize as u32,
                    // macos only, don't care
                    flags: 0,
                },
                0,
            )
        } else {
            warn!("{name:?} is not present");
            reply.error(ENOENT);
        }
    }

    // Seems to be entirely advisory. Default impl does nothing.
    // fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

    #[tracing::instrument(skip(self, _req, reply))]
    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        match self.mount_point.files.get(&ino) {
            Some(entry) => {
                let stat = &entry.stat;
                reply.attr(
                    &Duration::from_secs(1),
                    &FileAttr {
                        ino: stat.st_ino,
                        size: stat.st_size as u64,
                        blocks: stat.st_blocks as u64,
                        atime: SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_atime as u64),
                        mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_mtime as u64),
                        ctime: SystemTime::UNIX_EPOCH + Duration::from_secs(stat.st_ctime as u64),
                        // macos only, don't care
                        crtime: SystemTime::UNIX_EPOCH,
                        kind: stat_file_type(stat).unwrap(),
                        perm: stat.st_mode as u16,
                        nlink: stat.st_nlink as u32,
                        uid: stat.st_uid,
                        gid: stat.st_gid,
                        rdev: stat.st_rdev as u32,
                        blksize: stat.st_blksize as u32,
                        // macos only, don't care
                        flags: 0,
                    },
                );
            }
            _ => reply.error(ENOENT),
        }
    }

    // fn setattr(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     mode: Option<u32>,
    //     uid: Option<u32>,
    //     gid: Option<u32>,
    //     size: Option<u64>,
    //     _atime: Option<TimeOrNow>,
    //     _mtime: Option<TimeOrNow>,
    //     _ctime: Option<SystemTime>,
    //     fh: Option<u64>,
    //     _crtime: Option<SystemTime>,
    //     _chgtime: Option<SystemTime>,
    //     _bkuptime: Option<SystemTime>,
    //     flags: Option<u32>,
    //     reply: ReplyAttr,
    // ) {
    // }

    // fn readlink(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyData) {}

    // fn mknod(
    //     &mut self,
    //     _req: &Request<'_>,
    //     parent: u64,
    //     name: &OsStr,
    //     mode: u32,
    //     umask: u32,
    //     rdev: u32,
    //     reply: ReplyEntry,
    // ) {
    // }

    // fn mkdir(
    //     &mut self,
    //     _req: &Request<'_>,
    //     parent: u64,
    //     name: &OsStr,
    //     mode: u32,
    //     umask: u32,
    //     reply: ReplyEntry,
    // ) {
    // }

    // fn unlink(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {}

    // fn rmdir(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {}

    // fn symlink(
    //     &mut self,
    //     _req: &Request<'_>,
    //     parent: u64,
    //     link_name: &OsStr,
    //     target: &Path,
    //     reply: ReplyEntry,
    // ) {
    // }

    // fn rename(
    //     &mut self,
    //     _req: &Request<'_>,
    //     parent: u64,
    //     name: &OsStr,
    //     newparent: u64,
    //     newname: &OsStr,
    //     flags: u32,
    //     reply: ReplyEmpty,
    // ) {
    // }

    // fn link(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     newparent: u64,
    //     newname: &OsStr,
    //     reply: ReplyEntry,
    // ) {
    // }

    #[tracing::instrument(skip(self, _req, reply))]
    fn open(&mut self, _req: &Request<'_>, ino: u64, flags: i32, reply: ReplyOpen) {
        if let Some(path) = self.mount_point.path_from_inode(ino) {
            info!("Opening {} with flags {flags:#X}", path.display());
            let mut path = path.into_os_string().into_vec();
            path.push(0);
            let fd = unsafe { libc::openat(self.mount_point.fd, path.as_ptr().cast(), flags, 0) };
            if fd == -1 {
                let err = io::Error::last_os_error();
                if let Some(err) = err.raw_os_error() {
                    reply.error(err);
                } else {
                    // FIXME: Find a better fallback
                    reply.error(ENOENT);
                }
                error!("Couldn't open file: {err}",);
                return;
            }
            let idx = self.open_files.insert(unsafe { File::from_raw_fd(fd) });
            reply.opened(idx as u64, flags as u32);
        } else {
            info!("No entry for file");
            reply.error(ENOENT);
        }
    }

    #[tracing::instrument(skip(self, _req, offset, size, _flags, _lock, reply))]
    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        trace!("Reading {size} bytes at offset {offset}");

        if let Some(file) = self.open_files.get_mut(fh as usize) {
            if let Err(err) = file.seek(SeekFrom::Start(offset as u64)) {
                error!("Couldn't seek: {err}");
                let _ = err.raw_os_error().map(|err| reply.error(err));
                return;
            }
            // TODO: Re-use this allocation across reads
            let mut buf: Vec<u8> = iter::repeat(0).take(size as usize).collect();
            match file.read(&mut buf) {
                Ok(bytes_read) => {
                    reply.data(&buf[..bytes_read]);
                    return;
                }
                Err(err) => {
                    error!("Couldn't read: {err}");
                    let _ = err.raw_os_error().map(|err| reply.error(err));
                    return;
                }
            }
        }

        reply.error(ENOENT)
    }

    // fn write(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     offset: i64,
    //     data: &[u8],
    //     write_flags: u32,
    //     flags: i32,
    //     lock_owner: Option<u64>,
    //     reply: ReplyWrite,
    // ) {
    // }

    // fn flush(&mut self, _req: &Request<'_>, ino: u64, fh: u64, lock_owner: u64, reply: ReplyEmpty) {
    // } {}

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
        debug!("release {ino}");
        self.open_files.remove(fh as usize);
        reply.ok();
    }

    // fn fsync(&mut self, _req: &Request<'_>, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {}

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        // NOTE: This is the default implementation for this trait method.
        reply.opened(0, 0);
    }

    #[tracing::instrument(skip(self, _req, _fh, reply))]
    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        for (i, (name, entry)) in self
            .mount_point
            .read_dir(ino)
            .enumerate()
            .skip(offset as usize)
        {
            if reply.add(
                entry.stat.st_ino,
                (i + 1) as i64,
                stat_file_type(&entry.stat).unwrap(),
                name,
            ) {
                break;
            }
        }

        reply.ok();
    }

    // fn readdirplus(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     offset: i64,
    //     reply: ReplyDirectoryPlus,
    // ) {
    // }

    // fn releasedir(
    //     &mut self,
    //     _req: &Request<'_>,
    //     _ino: u64,
    //     fh: u64,
    //     _flags: i32,
    //     reply: ReplyEmpty,
    // ) {
    // }

    // fn fsyncdir(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     datasync: bool,
    //     reply: ReplyEmpty,
    // ) {
    // }

    // fn statfs(&mut self, _req: &Request<'_>, _ino: u64, reply: ReplyStatfs) {}

    // fn setxattr(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     name: &OsStr,
    //     _value: &[u8],
    //     flags: i32,
    //     position: u32,
    //     reply: ReplyEmpty,to
    // ) {
    // }

    // fn getxattr(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     name: &OsStr,
    //     size: u32,
    //     reply: ReplyXattr,
    // ) {
    // }

    // fn listxattr(&mut self, _req: &Request<'_>, ino: u64, size: u32, reply: ReplyXattr) {}

    // fn removexattr(&mut self, _req: &Request<'_>, ino: u64, name: &OsStr, reply: ReplyEmpty) {}

    // fn access(&mut self, _req: &Request<'_>, ino: u64, mask: i32, reply: ReplyEmpty) {}

    // fn create(
    //     &mut self,
    //     _req: &Request<'_>,
    //     parent: u64,
    //     name: &OsStr,
    //     mode: u32,
    //     umask: u32,
    //     flags: i32,
    //     reply: ReplyCreate,
    // ) {
    // }

    // fn getlk(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     lock_owner: u64,
    //     start: u64,
    //     end: u64,
    //     typ: i32,
    //     pid: u32,
    //     reply: ReplyLock,
    // ) {
    // }

    // fn setlk(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     lock_owner: u64,
    //     start: u64,
    //     end: u64,
    //     typ: i32,
    //     pid: u32,
    //     sleep: bool,
    //     reply: ReplyEmpty,
    // ) {
    // }

    // fn bmap(&mut self, _req: &Request<'_>, ino: u64, blocksize: u32, idx: u64, reply: ReplyBmap) {}

    // fn ioctl(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     flags: u32,
    //     cmd: u32,
    //     in_data: &[u8],
    //     out_size: u32,
    //     reply: ReplyIoctl,
    // ) {
    // }

    // fn fallocate(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     offset: i64,
    //     length: i64,
    //     mode: i32,
    //     reply: ReplyEmpty,
    // ) {
    // }

    // fn lseek(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino: u64,
    //     fh: u64,
    //     offset: i64,
    //     whence: i32,
    //     reply: ReplyLseek,
    // ) {
    // }

    // fn copy_file_range(
    //     &mut self,
    //     _req: &Request<'_>,
    //     ino_in: u64,
    //     fh_in: u64,
    //     offset_in: i64,
    //     ino_out: u64,
    //     fh_out: u64,
    //     offset_out: i64,
    //     len: u64,
    //     flags: u32,
    //     reply: ReplyWrite,
    // ) {
    // }
}

#[derive(Debug)]
struct MountPoint {
    fd: c_int,
    files: IndexMap<ino64_t, FileEntry>,
    dir_names: IndexMap<ino64_t, OsString>,
    in_dir_name_to_file: IndexMap<(ino64_t, OsString), ino64_t>,
}

#[derive(Debug)]
struct FileEntry {
    stat: stat64,
    // Only used to recover a useful path for `open` and the like.
    parent: ino64_t,
    // Only set for directories
    dir_contents: Option<Vec<OsString>>,
}

impl MountPoint {
    #[tracing::instrument]
    fn open(name: &str) -> io::Result<Self> {
        let mut name = name.to_owned();
        name.push('\0');
        let fd = unsafe { libc::open(name.as_ptr().cast(), libc::O_DIRECTORY | libc::O_PATH) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        let dir = unsafe { libc::opendir(name.as_ptr().cast()) };
        if dir.is_null() {
            return Err(io::Error::last_os_error());
        }
        let stat = unsafe { _fstat64(fd)? };
        name.pop();
        let mut files = IndexMap::new();
        files.insert(
            1,
            FileEntry {
                stat,
                dir_contents: None,
                parent: 0,
            },
        );
        let mut dir_names = IndexMap::new();
        dir_names.insert(1, name.clone().into());
        let mut in_dir_name_to_file = IndexMap::new();
        let mut directories = vec![(PathBuf::new(), 1)];
        while let Some((dir_name, dir_inode)) = directories.pop() {
            let span = debug_span!("", dir_name = format!("{}", dir_name.display()),);
            let _span = span.enter();
            let read_dir = if dir_inode == 1 {
                ReadDir::new(dir, fd)
            } else {
                let name = OsString::from_vec(
                    name.bytes()
                        .chain(iter::once(b'/'))
                        .chain(dir_name.as_os_str().as_bytes().iter().cloned())
                        .chain(iter::once(0))
                        .collect::<Vec<u8>>(),
                );
                debug!("Opening {}", Path::new(&name).display());
                let fd = unsafe {
                    libc::open(
                        name.as_bytes().as_ptr().cast(),
                        libc::O_DIRECTORY | libc::O_PATH,
                    )
                };
                if fd == -1 {
                    let err = io::Error::last_os_error();
                    error!("Could not open fd for {name:?}: {err}");
                    return Err(err);
                }
                let dir = unsafe { libc::opendir(name.as_bytes().as_ptr().cast()) };
                if dir.is_null() {
                    let err = io::Error::last_os_error();
                    error!("Could not open *mut DIR for {name:?}: {err}");
                    return Err(err);
                }
                ReadDir::new(dir, fd)
            };
            let mut dir_contents = Vec::new();
            for (name, stat) in read_dir {
                if FileType::Directory == stat_file_type(&stat).unwrap()
                    && name != "."
                    && name != ".."
                {
                    dir_names.insert(stat.st_ino, name.clone());
                    directories.push((dir_name.join(&name), stat.st_ino));
                }
                dir_contents.push(name.clone());
                in_dir_name_to_file.insert((dir_inode, name), stat.st_ino);
                files.entry(stat.st_ino).or_insert_with(|| FileEntry {
                    stat,
                    parent: dir_inode,
                    dir_contents: None,
                });
            }
            files.get_mut(&dir_inode).unwrap().dir_contents = Some(dir_contents);
        }
        Ok(Self {
            fd,
            files,
            dir_names,
            in_dir_name_to_file,
        })
    }

    fn read_dir(&self, inode: u64) -> impl Iterator<Item = (&OsString, &FileEntry)> {
        self.files
            .get(&inode)
            .and_then(|root| root.dir_contents.as_ref())
            .into_iter()
            .flat_map(move |dir_contents| {
                dir_contents.iter().flat_map(move |in_dir_name| {
                    self.in_dir_name_to_file
                        .get(&(inode, in_dir_name.clone()))
                        .and_then(|child_inode| self.files.get(child_inode))
                        .map(|file_entry| (in_dir_name, file_entry))
                })
            })
    }

    #[tracing::instrument(skip(self))]
    fn path_from_inode(&self, inode: u64) -> Option<PathBuf> {
        for ((entry_parent_dir, entry_name), entry_inode) in self.in_dir_name_to_file.iter() {
            if *entry_inode != inode {
                continue;
            }
            debug!("Found name for inode: {entry_name:?} with parent ino {entry_parent_dir}");
            let mut parents = vec![];
            let mut parent_inode = *entry_parent_dir;
            while let Some(parent_entry) = self.files.get(&parent_inode) {
                let span = debug_span!("", ino = parent_entry.stat.st_ino);
                let _span = span.enter();
                if parent_entry.stat.st_ino == 0 || parent_entry.stat.st_ino == 1 {
                    debug!("Skipping");
                    break;
                }
                debug!("Parent of {parent_inode} is {}", parent_entry.parent);
                parents.push(parent_entry.stat.st_ino);
                parent_inode = parent_entry.parent;
                continue;
            }
            return Some(
                parents
                    .into_iter()
                    .rev()
                    .flat_map(|parent| self.dir_names.get(&parent))
                    .chain(iter::once(entry_name))
                    .collect(),
            );
        }
        None
    }
}

unsafe impl Send for MountPoint {}

/// Adapted from `std` since I can't re-use the type :(
///
/// <https://github.com/rust-lang/rust/blob/master/library/std/src/sys/pal/unix/fs.rs#L690>
struct ReadDir {
    fd: c_int,
    dir: *mut libc::DIR,
    end_of_stream: bool,
}

impl ReadDir {
    fn new(dir: *mut libc::DIR, fd: c_int) -> ReadDir {
        Self {
            fd,
            dir,
            end_of_stream: false,
        }
    }
}

impl Iterator for ReadDir {
    type Item = (OsString, stat64);

    #[tracing::instrument(skip(self))]
    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.end_of_stream {
                return None;
            }
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir64(self.dir) };
            if entry.is_null() {
                self.end_of_stream = true;

                match unsafe { *libc::__errno_location() } {
                    0 => {}
                    e => error!(
                        "Error enumerating next directory entry: {}",
                        io::Error::from_raw_os_error(e)
                    ),
                };
                return None;
            }
            let name = unsafe {
                CStr::from_ptr(entry.cast::<i8>().add(offset_of!(libc::dirent64, d_name)))
            };
            let name = OsStr::from_bytes(name.to_bytes_with_nul()).to_owned();
            let stat = match unsafe { _fstatat64(self.fd, name.as_bytes().as_ptr().cast()) } {
                Some(stat) => {
                    if stat_file_type(&stat).is_none() {
                        // Discard entries whose file-type we can't pass along to `fuser`
                        continue;
                    } else {
                        stat
                    }
                }
                None => continue,
            };
            let mut name = name.into_vec();
            name.pop();
            let name = OsString::from_vec(name);
            debug!("Enumerating {name:?}");
            return Some((name, stat));
        }
    }
}

impl Drop for ReadDir {
    fn drop(&mut self) {
        let code = unsafe { libc::closedir(self.dir) };
        if code == -1 {
            error!("Error closing directory: {}", io::Error::last_os_error());
        }
    }
}

fn stat_file_type(stat: &stat64) -> Option<FileType> {
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFBLK => Some(FileType::BlockDevice),
        libc::S_IFCHR => Some(FileType::CharDevice),
        libc::S_IFDIR => Some(FileType::Directory),
        libc::S_IFIFO => Some(FileType::NamedPipe),
        libc::S_IFREG => Some(FileType::RegularFile),
        libc::S_IFSOCK => Some(FileType::Socket),
        _ => None,
    }
}

unsafe fn _fstatat64(fd: c_int, name: *const i8) -> Option<stat64> {
    let mut stat = mem::zeroed::<libc::stat64>();
    let code = libc::fstatat64(fd, name, &mut stat, libc::AT_SYMLINK_NOFOLLOW);
    if code == -1 {
        error!(
            "Error calling fstatat64 on {:?}: {}",
            CStr::from_ptr(name),
            io::Error::last_os_error()
        );
        return None;
    }
    Some(stat)
}

unsafe fn _fstat64(fd: c_int) -> io::Result<stat64> {
    let mut stat = mem::zeroed::<libc::stat64>();
    let code = libc::fstat64(fd, &mut stat);
    if code == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(stat)
}
