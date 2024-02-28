use std::{
    ffi::{c_int, c_uchar, CStr, CString, OsStr, OsString},
    io,
    mem::{self, offset_of},
    os::unix::ffi::OsStrExt,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, UNIX_EPOCH},
};

use color_eyre::eyre;
use fuser::{
    FileAttr, FileType, Filesystem, KernelConfig, ReplyAttr, ReplyData, ReplyDirectory, ReplyEntry,
    ReplyOpen, Request,
};
use libc::ENOENT;
use tracing::{error, info, warn};

static EXIT: AtomicBool = AtomicBool::new(false);

pub fn launch() -> eyre::Result<()> {
    ctrlc::set_handler(|| EXIT.store(true, Ordering::SeqCst))?;
    let mount_point = unsafe { MountPoint::open("./gaem\0")? };
    let _handle = fuser::spawn_mount2(ModdingFileSystem::new(mount_point), "./gaem", &[])?;

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
}

impl ModdingFileSystem {
    fn new(mount_point: MountPoint) -> Self {
        Self { mount_point }
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

    #[tracing::instrument(skip_all)]
    fn lookup(&mut self, _req: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        info!("lookup name: {name:?}");

        if parent == 1 && name.to_str() == Some("hello.txt") {
            reply.entry(
                &Duration::from_secs(1),
                &FileAttr {
                    ino: 2,
                    size: 13,
                    blocks: 1,
                    atime: UNIX_EPOCH,
                    mtime: UNIX_EPOCH,
                    ctime: UNIX_EPOCH,
                    crtime: UNIX_EPOCH,
                    kind: FileType::RegularFile,
                    perm: 0o644,
                    nlink: 1,
                    uid: 501,
                    gid: 20,
                    rdev: 0,
                    flags: 0,
                    blksize: 512,
                },
                0,
            );
        } else if self
            .mount_point
            .read_dir()
            .filter_map(|e| e.ok())
            .any(|(entry_name, _)| {
                // chop off the null terminator before comparing
                let entry_name = entry_name.as_bytes();
                let entry_name = OsStr::from_bytes(&entry_name[0..entry_name.len() - 1]);
                entry_name == name
            })
        {
            info!("here");
            reply.entry(
                &Duration::from_secs(1),
                &FileAttr {
                    ino: 3,
                    size: 13,
                    blocks: 1,
                    atime: UNIX_EPOCH,
                    mtime: UNIX_EPOCH,
                    ctime: UNIX_EPOCH,
                    crtime: UNIX_EPOCH,
                    kind: FileType::RegularFile,
                    perm: 0o644,
                    nlink: 1,
                    uid: 501,
                    gid: 20,
                    rdev: 0,
                    flags: 0,
                    blksize: 512,
                },
                0,
            );
        } else {
            info!("{name:?} is not present");
            reply.error(ENOENT);
        }
    }

    // Seems to be entirely advisory. Default impl does nothing.
    // fn forget(&mut self, _req: &Request<'_>, _ino: u64, _nlookup: u64) {}

    #[tracing::instrument(skip_all)]
    fn getattr(&mut self, _req: &Request<'_>, ino: u64, reply: ReplyAttr) {
        info!("getattr ino: {ino}");

        match ino {
            1 => reply.attr(
                &Duration::from_secs(1),
                &FileAttr {
                    ino: 1,
                    size: 0,
                    blocks: 0,
                    atime: UNIX_EPOCH,
                    mtime: UNIX_EPOCH,
                    ctime: UNIX_EPOCH,
                    crtime: UNIX_EPOCH,
                    kind: FileType::Directory,
                    perm: 0o755,
                    nlink: 2,
                    uid: 1000,
                    gid: 20,
                    rdev: 0,
                    blksize: 512,
                    flags: 0,
                },
            ),
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

    #[tracing::instrument(skip_all)]
    fn open(&mut self, _req: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        info!("open {ino}");
        reply.opened(0, 0);
    }

    #[tracing::instrument(skip_all)]
    fn read(
        &mut self,
        _req: &Request,
        ino: u64,
        _fh: u64,
        offset: i64,
        _size: u32,
        _flags: i32,
        _lock: Option<u64>,
        reply: ReplyData,
    ) {
        info!("Reading ino: {ino}");
        if ino == 2 {
            reply.data(&"Hello World!\n".as_bytes()[offset as usize..]);
        } else {
            reply.error(ENOENT);
        }
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

    // fn release(
    //     &mut self,
    //     _req: &Request<'_>,
    //     _ino: u64,
    //     _fh: u64,
    //     _flags: i32,
    //     _lock_owner: Option<u64>,
    //     _flush: bool,
    //     reply: ReplyEmpty,
    // ) {
    // }

    // fn fsync(&mut self, _req: &Request<'_>, ino: u64, fh: u64, datasync: bool, reply: ReplyEmpty) {}

    fn opendir(&mut self, _req: &Request<'_>, _ino: u64, _flags: i32, reply: ReplyOpen) {
        // NOTE: This is the default implementation for this trait method.
        reply.opened(0, 0);
    }

    #[tracing::instrument(skip_all)]
    fn readdir(
        &mut self,
        _req: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino != 1 {
            reply.error(ENOENT);
            return;
        }

        let read_dir = self.mount_point.read_dir();

        let entries = [
            (1, FileType::Directory, "."),
            (1, FileType::Directory, ".."),
            (2, FileType::RegularFile, "hello.txt"),
        ];

        for (i, entry) in entries.into_iter().enumerate().skip(offset as usize) {
            // i + 1 means the index of the next entry
            if reply.add(entry.0, (i + 1) as i64, entry.1, entry.2) {
                break;
            }
        }
        for (i, entry) in read_dir.enumerate().skip(offset as usize) {
            match entry {
                Ok((name, d_type)) => {
                    if reply.add(i as u64 + 3, (i + 1) as i64, d_type, name) {
                        break;
                    }
                }
                Err(err) => error!("Error enumerating directory entry: {err}"),
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
    //     _fh: u64,
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
    //     reply: ReplyEmpty,
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
    dir: *mut libc::DIR,
}

impl MountPoint {
    unsafe fn open(name: &str) -> io::Result<Self> {
        let fd = unsafe { libc::open(name.as_ptr().cast(), libc::O_DIRECTORY | libc::O_PATH) };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        let dir = unsafe { libc::opendir(name.as_ptr().cast()) };
        if dir.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { fd, dir })
    }

    fn read_dir(&self) -> ReadDir {
        unsafe { libc::rewinddir(self.dir) };
        ReadDir::new(self.dir, self.fd)
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
    type Item = io::Result<(OsString, FileType)>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.end_of_stream {
                return None;
            }
            unsafe { *libc::__errno_location() = 0 };
            let entry = unsafe { libc::readdir64(self.dir) };
            if entry.is_null() {
                self.end_of_stream = true;

                return match unsafe { *libc::__errno_location() } {
                    0 => None,
                    e => Some(Err(io::Error::from_raw_os_error(e))),
                };
            }
            let name = unsafe {
                CStr::from_ptr(entry.cast::<i8>().add(offset_of!(libc::dirent64, d_name)))
            };
            let name = OsStr::from_bytes(name.to_bytes_with_nul()).to_owned();
            let d_type = unsafe {
                *entry
                    .cast::<u8>()
                    .add(offset_of!(libc::dirent64, d_type))
                    .cast::<c_uchar>()
            };
            let file_type = match d_type {
                libc::DT_BLK => Some(FileType::BlockDevice),
                libc::DT_CHR => Some(FileType::CharDevice),
                libc::DT_DIR => Some(FileType::Directory),
                libc::DT_FIFO => Some(FileType::NamedPipe),
                libc::DT_LNK => Some(FileType::Symlink),
                libc::DT_REG => Some(FileType::RegularFile),
                libc::DT_SOCK => Some(FileType::Socket),
                libc::DT_UNKNOWN => unsafe {
                    let mut stat = mem::zeroed::<libc::stat>();
                    let code = libc::fstatat(
                        self.fd,
                        name.as_bytes().as_ptr().cast(),
                        &mut stat,
                        libc::AT_SYMLINK_NOFOLLOW,
                    );
                    if code == -1 {
                        error!(
                            "Error getting file type/mode: {}",
                            io::Error::last_os_error()
                        );
                    }
                    match stat.st_mode {
                        libc::S_IFBLK => Some(FileType::BlockDevice),
                        libc::S_IFCHR => Some(FileType::CharDevice),
                        libc::S_IFDIR => Some(FileType::Directory),
                        libc::S_IFIFO => Some(FileType::NamedPipe),
                        libc::S_IFREG => Some(FileType::RegularFile),
                        libc::S_IFSOCK => Some(FileType::Socket),
                        _ => None,
                    }
                },
                _ => {
                    error!("Unknown d_type: {d_type}");
                    None
                }
            };
            if let Some(file_type) = file_type {
                return Some(Ok((name, file_type)));
            }
        }
    }
}
