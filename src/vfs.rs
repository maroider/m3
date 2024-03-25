use std::{
    any::Any,
    ffi::OsString,
    os::linux::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use slab::Slab;
use tracing::{debug, error};

// TODO: Add some way of mounting a layer at a particular path other than the root

pub struct Vfs {
    layers: Slab<Box<dyn Layer>>,
    // NOTE: Not used yet
    order: Vec<usize>,
}

impl std::fmt::Debug for Vfs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Vfs")
            .field("layers", &[(); 0])
            .field("order", &self.order)
            .finish()
    }
}

pub struct DirEntry {
    pub name: OsString,
    pub typ: FileType,
}

pub enum FileType {
    Dir,
    File,
    Symlink,
}

#[allow(private_bounds)]
impl Vfs {
    pub fn new() -> Self {
        Self {
            layers: Slab::new(),
            order: Vec::new(),
        }
    }

    pub fn push_layer<L>(&mut self, layer: L)
    where
        L: Layer,
    {
        let _idx = self.layers.insert(Box::new(layer));
    }

    // FIXME: None of the VFS querying functions handle the case where a name may be a dir in one
    // layer, and a file in another layer.
    pub fn read_dir<'a, P>(&'a self, dir: P) -> Option<impl Iterator<Item = DirEntry> + 'a>
    where
        P: AsRef<Path> + 'a,
    {
        if !dir.as_ref().is_absolute() {
            panic!(r#"{} does not start with a "/""#, dir.as_ref().display());
        }
        if !self
            .layers
            .iter()
            .any(|(_, layer)| layer.as_ref().dir_exists(dir.as_ref()))
        {
            return None;
        }
        Some(self.layers.iter().flat_map(move |(_, layer)| {
            let mut dirs = Vec::new();
            layer.read_dir(dir.as_ref(), &mut |entry| dirs.push(entry));
            dirs.into_iter()
        }))
    }

    pub fn file_attr<P>(&self, path: P) -> Option<fuser::FileAttr>
    where
        P: AsRef<Path>,
    {
        if !path.as_ref().is_absolute() {
            panic!(r#"{} does not start with a "/""#, path.as_ref().display());
        }
        for (_, layer) in self.layers.iter().rev() {
            if let Some(attrs) = layer.file_attr(path.as_ref()) {
                return Some(attrs);
            }
        }
        None
    }

    pub fn layer_that_opens_file<P>(&self, file: P) -> Option<&dyn Layer>
    where
        P: AsRef<Path>,
    {
        let file = file.as_ref();
        if !file.is_absolute() {
            panic!(r#"{} does not start with a "/""#, file.display());
        }
        for (_, layer) in self.layers.iter().rev() {
            if layer.file_exists(file) {
                return Some(layer.as_ref());
            }
        }
        None
    }
}

pub trait Layer: Any + Send + 'static {
    fn as_any(&self) -> &(dyn Any + 'static);
    fn dir_exists(&self, dir: &Path) -> bool;
    fn file_exists(&self, file: &Path) -> bool;
    fn read_dir(&self, dir: &Path, callback: &mut dyn FnMut(DirEntry));
    fn file_attr(&self, file: &Path) -> Option<fuser::FileAttr>;
}

struct RawFsLayer {
    source: PathBuf,
}

impl RawFsLayer {
    pub fn new<P>(source: P) -> Self
    where
        P: AsRef<Path>,
    {
        Self {
            source: source.as_ref().to_owned(),
        }
    }
}

impl Layer for RawFsLayer {
    fn as_any(&self) -> &(dyn Any + 'static) {
        self
    }

    fn dir_exists(&self, dir: &Path) -> bool {
        self.source.join(dir).is_dir()
    }

    fn file_exists(&self, file: &Path) -> bool {
        self.source.join(file).is_file()
    }

    #[tracing::instrument(skip(self, callback))]
    fn read_dir(&self, dir: &Path, callback: &mut dyn FnMut(DirEntry)) {
        let dir = self.source.join(dir);
        debug!("{}", dir.display());
        let rdir = match std::fs::read_dir(&dir) {
            Ok(rdir) => rdir,
            Err(err) => {
                error!("{err}");
                return;
            }
        };
        for subdir in rdir {
            match subdir {
                Ok(dirent) => {
                    let name = dirent.file_name();
                    let typ = match dirent.file_type() {
                        Ok(typ) => typ,
                        Err(err) => {
                            error!("Could not get file type for {:?}: {err}", name);
                            continue;
                        }
                    };
                    let typ = if typ.is_dir() {
                        FileType::Dir
                    } else if typ.is_file() {
                        FileType::File
                    } else if typ.is_symlink() {
                        FileType::Symlink
                    } else {
                        error!("{:?} has an unsupported file type", dirent.file_name());
                        continue;
                    };
                    let entry = DirEntry { name, typ };
                    callback(entry);
                }
                Err(err) => {
                    error!("{err}");
                }
            }
        }
    }

    fn file_attr(&self, file: &Path) -> Option<fuser::FileAttr> {
        let file = self.source.join(file);
        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(err) => {
                error!("Could not get metadata for {}: {err}", file.display());
                return None;
            }
        };
        let kind = if metadata.is_dir() {
            fuser::FileType::Directory
        } else if metadata.is_file() {
            fuser::FileType::RegularFile
        } else if metadata.is_symlink() {
            fuser::FileType::Symlink
        } else {
            return None;
        };
        Some(fuser::FileAttr {
            // This will get changed to something meaningful elsewhere
            ino: 0,
            size: metadata.st_size(),
            blocks: metadata.st_blocks(),
            atime: SystemTime::UNIX_EPOCH + Duration::from_secs(metadata.st_atime() as u64),
            mtime: SystemTime::UNIX_EPOCH + Duration::from_secs(metadata.st_mtime() as u64),
            ctime: SystemTime::UNIX_EPOCH + Duration::from_secs(metadata.st_ctime() as u64),
            crtime: SystemTime::UNIX_EPOCH,
            kind,
            perm: metadata.st_mode() as u16,
            nlink: metadata.st_nlink() as u32,
            uid: metadata.st_uid(),
            gid: metadata.st_gid(),
            rdev: metadata.st_rdev() as u32,
            blksize: metadata.st_blksize() as u32,
            flags: 0,
        })
    }
}
