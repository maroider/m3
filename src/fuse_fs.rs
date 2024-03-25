use std::{
    any::{type_name_of_val, Any},
    collections::HashMap,
    ffi::{c_int, CStr, OsStr, OsString},
    fmt::Debug,
    fs::File,
    io::{self, Read, Seek, SeekFrom, Write},
    iter,
    mem::{self, offset_of},
    os::{
        fd::FromRawFd,
        unix::ffi::{OsStrExt, OsStringExt},
    },
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, SystemTime},
};

use color_eyre::eyre;
use fuser::{
    FileType, Filesystem, KernelConfig, ReplyAttr, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, Request,
};
use libc::{ino64_t, stat64, ENOENT};
use slab::Slab;
use tracing::{debug, debug_span, error, info, trace, warn};

use crate::vfs::{self, Vfs};

static EXIT: AtomicBool = AtomicBool::new(false);

pub fn launch() -> eyre::Result<()> {
    ctrlc::set_handler(|| EXIT.store(true, Ordering::SeqCst))?;
    let path = "./testdir/gamedir";
    let _handle = fuser::spawn_mount2(ModdingFileSystem::new(path)?, path, &[])?;

    loop {
        thread::sleep(Duration::from_millis(100));
        if EXIT.load(Ordering::SeqCst) {
            break;
        }
    }

    Ok(())
}

pub trait WriteSeek: Read + Write + Seek + Send {}
impl<W> WriteSeek for W where W: Read + Write + Seek + Send {}

struct ModdingFileSystem {
    vfs: Vfs,
    inode_to_path: HashMap<ino64_t, PathBuf>,
    path_inodes: HashMap<PathBuf, ino64_t>,
    next_inode: ino64_t,
    open_files: Slab<Box<dyn WriteSeek>>,
}

impl Debug for ModdingFileSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModdingFileSystem")
            .field("vfs", &self.vfs)
            .field("inode_to_path", &self.inode_to_path)
            .field("path_inodes", &self.path_inodes)
            .field("next_inode", &self.next_inode)
            .field("open_files", &[(); 0])
            .finish()
    }
}

impl ModdingFileSystem {
    fn new<P>(path: &P) -> eyre::Result<Self>
    where
        P: AsRef<Path> + Debug + ?Sized,
    {
        let mut inode_to_path = HashMap::new();
        inode_to_path.insert(1, "/".into());
        let mut path_inodes = HashMap::new();
        path_inodes.insert("/".into(), 1);
        let mut next_inode = 2;
        let mount_point = MountPoint::open(path, &mut next_inode)?;
        let mut vfs = Vfs::new();
        vfs.push_layer(mount_point);
        Ok(Self {
            vfs,
            inode_to_path,
            path_inodes,
            next_inode,
            open_files: Slab::new(),
        })
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
        let Some(parent_path) = self.inode_to_path.get(&parent) else {
            error!("No path registered for inode {parent}");
            reply.error(ENOENT);
            return;
        };

        let Some(entry) = self
            .vfs
            .read_dir(parent_path)
            .and_then(|mut readdir| readdir.find(|entry| entry.name == name))
        else {
            warn!("{name:?} is not present");
            reply.error(ENOENT);
            return;
        };

        // FIXME: Factor out the second hashmap lookup here
        let item_path = parent_path.join(entry.name);
        let Some(attrs) = self.vfs.file_attr(&item_path) else {
            error!("Couldn't get file attributes for {}", item_path.display());
            reply.error(ENOENT);
            return;
        };

        info!("{name:?} is ino {}", attrs.ino);
        self.inode_to_path.insert(attrs.ino, item_path);
        reply.entry(&Duration::from_secs(1), &attrs, 0);
    }

    // Seems to be entirely advisory. Default impl does nothing.
    // fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

    #[tracing::instrument(skip(self, _req, reply))]
    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        if let Some(path) = self.inode_to_path.get(&ino) {
            debug!("ino present as {}", path.display());
            if let Some(attrs) = self.vfs.file_attr(path) {
                debug!("attrs exist");
                reply.attr(&Duration::from_secs(120), &attrs);
                return;
            }
        }
        error!("Not present");
        reply.error(ENOENT);
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
        info!("");
        let path = self.inode_to_path.get(&ino).unwrap();
        info!("{}", path.display());
        let Some(layer) = self.vfs.layer_that_opens_file(path) else {
            reply.error(ENOENT);
            error!("no layer can open the given file");
            return;
        };
        if let Some(mount_point) = layer.as_any().downcast_ref::<MountPoint>() {
            info!("Layer found: {}", type_name_of_val(mount_point));
            info!("Opening {}", path.display());
            let path = path
                .as_os_str()
                .as_bytes()
                .iter()
                .skip(1) // Remove the leading /
                .cloned()
                .chain(iter::once(0))
                .collect::<Vec<_>>();
            let fd = unsafe { libc::openat(mount_point.fd, path.as_ptr().cast(), flags, 0) };
            if fd == -1 {
                let err = io::Error::last_os_error();
                error!("Could not open file: {err}");
                if let Some(err) = err.raw_os_error() {
                    reply.error(err);
                } else {
                    // FIXME: Not sure what to return here
                    error!("No OS error");
                    reply.error(ENOENT);
                }
                return;
            }
            let file = unsafe { File::from_raw_fd(fd) };
            let idx = self.open_files.insert(Box::new(file));
            reply.opened(idx as u64, flags as u32);
        } else {
            error!("Unhandled layer type");
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
        info!("");
        // TODO: Use the VFS instead
        let Some(path) = self.inode_to_path.get(&ino) else {
            // TODO: Make sure this is a reasonable error value for this case?
            // Granted, I don't think it's particularly likely that we'll hit this branch without
            // skill issue on my end.
            warn!("Could not find path for ino {ino}");
            reply.error(libc::ENOENT);
            return;
        };

        let Some(readdir) = self.vfs.read_dir(path) else {
            warn!("Couldn't enumerate {ino}: {}", path.display());
            return;
        };

        let mut new_inode_to_path = Vec::new();
        for (i, entry) in readdir.skip(offset as usize).enumerate() {
            let path = path.join(&entry.name);
            // We assume there's no mapping, and that this is the only place we need to "materialize"
            // the mapping.
            // FIXME: Make the file attributes respect the mapping...
            info!("Enumerating {}", path.display());
            let inode = self.path_inodes.entry(path.clone()).or_insert_with(|| {
                let inode = self.next_inode;
                new_inode_to_path.push((inode, path));
                self.next_inode += 1;
                inode
            });
            if reply.add(
                *inode,
                (i + 1) as i64,
                match entry.typ {
                    vfs::FileType::Dir => FileType::Directory,
                    vfs::FileType::File => FileType::RegularFile,
                    vfs::FileType::Symlink => FileType::Symlink,
                },
                entry.name,
            ) {
                break;
            }
        }
        self.inode_to_path.extend(new_inode_to_path);

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
    files: Vec<FileEntry>,
}

#[derive(Debug)]
struct FileEntry {
    stat: stat64,
    // Only set for directories
    dir_contents: Option<Vec<(usize, OsString)>>,
}

impl MountPoint {
    #[tracing::instrument]
    fn open<P>(name: P, next_inode: &mut ino64_t) -> io::Result<Self>
    where
        P: AsRef<Path> + Debug,
    {
        // A lot of this can likely be removed once https://github.com/rust-lang/rust/issues/120426
        // gets implemented.
        let mut name = name.as_ref().to_owned().into_os_string().into_vec();
        name.push(0);
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
        let mut files = Vec::new();
        files.push(FileEntry {
            stat,
            dir_contents: None,
        });
        let mut directories = vec![(PathBuf::new(), 0)];
        while let Some((dir_name, dir_idx)) = directories.pop() {
            let span = debug_span!("", dir_name = format!("{}", dir_name.display()),);
            let _span = span.enter();
            let read_dir = if dir_idx == 0 {
                ReadDir::new(dir, fd)
            } else {
                let name = OsString::from_vec(
                    name.iter()
                        .cloned()
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
            for (name, mut stat) in read_dir {
                if FileType::Directory == stat_file_type(&stat).unwrap() {
                    directories.push((dir_name.join(&name), files.len()));
                }
                dir_contents.push((files.len(), name.clone()));
                stat.st_ino = *next_inode;
                *next_inode += 1;
                files.push(FileEntry {
                    stat,
                    dir_contents: None,
                });
            }
            files.get_mut(dir_idx).unwrap().dir_contents = Some(dir_contents);
        }
        Ok(Self { fd, files })
    }

    fn find_entry_idx_for_path(&self, path: &Path) -> Option<usize> {
        let mut current_entry_idx = 0;
        for component in path.components() {
            if let Component::Normal(name) = component {
                let current_entry = &self.files[current_entry_idx];
                if let Some(dir_contents) = &current_entry.dir_contents {
                    if let Some((idx, _)) = dir_contents.iter().find(|(_, n)| n == name) {
                        current_entry_idx = *idx;
                        continue;
                    }
                }
                // The directory doesn't exist
                return None;
            }
        }
        Some(current_entry_idx)
    }
}

unsafe impl Send for MountPoint {}

impl vfs::Layer for MountPoint {
    fn as_any(&self) -> &(dyn Any + 'static) {
        self
    }

    fn dir_exists(&self, dir: &Path) -> bool {
        self.find_entry_idx_for_path(dir)
            .map(|entry_idx| self.files[entry_idx].dir_contents.is_some())
            .unwrap_or(false)
    }

    fn file_exists(&self, file: &Path) -> bool {
        self.find_entry_idx_for_path(file)
            .map(|idx| self.files[idx].dir_contents.is_none())
            .unwrap_or(false)
    }

    fn read_dir(&self, dir: &Path, callback: &mut dyn FnMut(vfs::DirEntry)) {
        let Some(entry_idx) = self.find_entry_idx_for_path(dir) else {
            return;
        };
        let current_entry = &self.files[entry_idx];
        if let Some(dir_contents) = &current_entry.dir_contents {
            for (entry_idx, entry_name) in dir_contents {
                let file = &self.files[*entry_idx];
                let Some(typ) = stat_vfs_file_type(&file.stat) else {
                    warn!("Unsupported filetype {}", file.stat.st_mode & libc::S_IFMT);
                    continue;
                };
                let entry = vfs::DirEntry {
                    name: entry_name.to_owned(),
                    typ,
                };
                callback(entry);
            }
        }
    }

    fn file_attr(&self, file: &Path) -> Option<fuser::FileAttr> {
        self.find_entry_idx_for_path(file).map(|idx| {
            let file = &self.files[idx];
            let stat = &file.stat;
            fuser::FileAttr {
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
            }
        })
    }
}

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
            // Are "." and ".." guaranteed to always be directory names? I would assume no
            // reasonable filesystem would let you assign these names to something else (maybe NTFS
            // would, idk), but it's probably not an issue in practice.
            if name == c"." || name == c".." {
                continue;
            }
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
        libc::S_IFLNK => Some(FileType::Symlink),
        libc::S_IFREG => Some(FileType::RegularFile),
        libc::S_IFSOCK => Some(FileType::Socket),
        _ => None,
    }
}

fn stat_vfs_file_type(stat: &stat64) -> Option<vfs::FileType> {
    match stat.st_mode & libc::S_IFMT {
        libc::S_IFDIR => Some(vfs::FileType::Dir),
        libc::S_IFLNK => Some(vfs::FileType::Symlink),
        libc::S_IFREG => Some(vfs::FileType::File),
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
