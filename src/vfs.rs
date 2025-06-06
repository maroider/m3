use std::{
    any::Any,
    collections::HashSet,
    ffi::OsString,
    iter,
    os::linux::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use slab::Slab;
use tracing::{debug, debug_span, error};

// TODO: Add some way of mounting a layer at a particular path other than the root

#[derive(Debug)]
pub struct Vfs {
    layers: Slab<LayerAt>,
    // NOTE: Not used yet
}

pub struct LayerAt {
    pub at: Option<PathBuf>,
    pub whitelist: Option<HashSet<OsString>>,
    pub layer: Box<dyn Layer>,
}

impl std::fmt::Debug for LayerAt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LayerAt")
            .field("at", &self.at)
            .field("layer", &())
            .finish()
    }
}

impl LayerAt {
    /// Returns `None` when the path can't be mapped, which usually means you should ignore the
    /// layer.
    fn map_path_at<'a>(&self, path: &'a Path) -> Option<&'a Path> {
        if let Some(at) = &self.at {
            let to = path.strip_prefix(at).ok();
            if let Some(to) = &to {
                debug!("mapped {} to {}", path.display(), to.display());
            }
            to
        } else {
            Some(path)
        }
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
        }
    }

    pub fn push_layer<L>(
        &mut self,
        layer: L,
        at: Option<&Path>,
        whitelist: Option<HashSet<OsString>>,
    ) where
        L: Layer,
    {
        let _idx = self.layers.insert(LayerAt {
            at: at.map(|at| at.to_path_buf()),
            whitelist,
            layer: Box::new(layer),
        });
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
        if !self.layers.iter().any(|(_, layer_at)| {
            let Some(dir) = layer_at.map_path_at(dir.as_ref()) else {
                return false;
            };
            layer_at.layer.as_ref().dir_exists(dir.as_ref())
        }) {
            return None;
        }
        let mut enumerated = HashSet::new();
        let mut layers = self.layers.iter();
        let mut layer_opt = layers.next().map(|(_, l)| l);
        let mut to_yield = Vec::new();
        Some(iter::from_fn(move || {
            let _span = debug_span!("read_dir iterator").entered();
            if to_yield.is_empty() {
                while let Some(layer_at) = layer_opt {
                    let Some(mapped_dir) = layer_at.map_path_at(dir.as_ref()) else {
                        layer_opt = layers.next().map(|(_, l)| l);
                        debug!("skipping layer");
                        continue;
                    };
                    layer_at.layer.read_dir(mapped_dir, &mut |entry| {
                        if let Some(whitelist) = layer_at.whitelist.as_ref() {
                            if !whitelist.contains(&entry.name) {
                                return;
                            }
                        }
                        if enumerated.insert(entry.name.clone()) {
                            to_yield.push(entry);
                        }
                    });
                    layer_opt = layers.next().map(|(_, l)| l);
                }
            }
            to_yield.pop()
        }))
    }

    pub fn file_attr<P>(&self, path: P) -> Option<fuser::FileAttr>
    where
        P: AsRef<Path>,
    {
        if !path.as_ref().is_absolute() {
            panic!(r#"{} does not start with a "/""#, path.as_ref().display());
        }
        for (_, layer_at) in self.layers.iter().rev() {
            let Some(path) = layer_at.map_path_at(path.as_ref()) else {
                continue;
            };
            if let Some(attrs) = layer_at.layer.file_attr(path) {
                return Some(attrs);
            }
        }
        None
    }

    pub fn layer_that_opens_file<'a>(&self, file: &'a Path) -> Option<(&'a Path, &dyn Layer)> {
        if !file.is_absolute() {
            panic!(r#"{} does not start with a "/""#, file.display());
        }
        for (_, layer_at) in self.layers.iter().rev() {
            let Some(file) = layer_at.map_path_at(file) else {
                continue;
            };
            if layer_at.layer.file_exists(file) {
                return Some((file, layer_at.layer.as_ref()));
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

pub struct RawFsLayer {
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

    pub fn source(&self) -> &Path {
        &self.source
    }
}

impl Layer for RawFsLayer {
    fn as_any(&self) -> &(dyn Any + 'static) {
        self
    }

    fn dir_exists(&self, dir: &Path) -> bool {
        let dir = dir.strip_prefix("/").unwrap_or(dir);
        self.source.join(dir).is_dir()
    }

    fn file_exists(&self, file: &Path) -> bool {
        let file = file.strip_prefix("/").unwrap_or(file);
        self.source.join(file).is_file()
    }

    #[tracing::instrument(skip(self, callback))]
    fn read_dir(&self, dir: &Path, callback: &mut dyn FnMut(DirEntry)) {
        let dir = self.source.join(dir.strip_prefix("/").unwrap_or(dir));
        debug!("dir: {}", dir.display());
        let rdir = match std::fs::read_dir(dir) {
            Ok(rdir) => rdir,
            Err(err) => {
                error!("Could not open directory for content enumeration: {err}");
                return;
            }
        };
        for subdir in rdir {
            match subdir {
                Ok(dirent) => {
                    let name = dirent.file_name();
                    debug!("Enumerating {name:?}");
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
                    error!("Failed to enumerate: {err}");
                }
            }
        }
    }

    #[tracing::instrument(skip(self))]
    fn file_attr(&self, file: &Path) -> Option<fuser::FileAttr> {
        let file = file.strip_prefix("/").unwrap_or(file);
        let file = self.source.join(file);
        debug!("{}", file.display());
        let metadata = match file.metadata() {
            Ok(m) => m,
            Err(err) => {
                debug!("Could not get metadata for {}: {err}", file.display());
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
