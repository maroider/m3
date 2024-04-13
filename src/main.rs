#![feature(panic_update_hook)]

use std::{
    borrow::Cow,
    fs::{self, OpenOptions},
    io::{self, Read},
    panic,
    path::Path,
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

use color_eyre::eyre;
use directories::ProjectDirs;
use fuse_fs::ModdingFileSystem;
use kdl::{KdlDocument, KdlNode};
use once_cell::sync::Lazy;
use tracing::{debug, error, info, level_filters::LevelFilter};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, Layer};

mod fuse_fs;
mod vfs;

fn main() {
    let logs_dir = PROJECT_DIRS.data_local_dir().join("logs");
    fs::create_dir_all(&logs_dir).unwrap();
    let crash_file = logs_dir.join("m3_crash.log");
    panic::update_hook(move |prev, info| {
        error!("Panic occured, check m3_crash.log for info");
        let _ = fs::write(
            &crash_file,
            format!(
                "panic occured at {:?} with payload: {:?}",
                info.location(),
                info.payload().downcast_ref::<&str>()
            ),
        );
        prev(info);
    });
    let file_appender = tracing_appender::rolling::never(&logs_dir, "m3.log");
    let collector = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::Layer::new().with_writer(io::stderr))
        .with(
            tracing_subscriber::fmt::Layer::new()
                .with_writer(file_appender)
                .with_filter(LevelFilter::DEBUG),
        );
    collector.init();
    info!("Logging initialized");

    color_eyre::install().unwrap();
    if let Err(e) = run() {
        error!("main function failed: {e}");
    }
}

pub fn run() -> eyre::Result<()> {
    let cfg = Config::new()?;
    let mut args = std::env::args();
    args.next();
    if let Some(exe) = args.next() {
        info!("Launched normally");
        let cwd = std::env::current_dir().unwrap();
        debug!("cwd = {}", cwd.display());
        debug!("config =\n{:#}", cfg.doc);
        let install = cfg
            .installs()
            .find(|install| {
                info!("install.target() = {}", install.target().display());
                install.target() == cwd
            })
            .unwrap();
        debug!("Install found for '{}'", install.target().display());
        info!("Launching FUSE fs");
        let _handle =
            fuser::spawn_mount2(ModdingFileSystem::new(&install)?, install.target(), &[])?;
        info!("Launching game '{}' with args: '{:?}'", exe, args);
        let mut game = Command::new(exe).args(args).spawn()?;
        info!("Waiting for game to exit");
        game.wait()?;
    } else {
        info!("Launched in dev mode");
        let install = cfg.installs().next().unwrap();
        let _handle =
            fuser::spawn_mount2(ModdingFileSystem::new(&install)?, install.target(), &[])?;

        static EXIT: AtomicBool = AtomicBool::new(false);
        ctrlc::set_handler(|| EXIT.store(true, Ordering::SeqCst))?;
        loop {
            thread::sleep(Duration::from_millis(100));
            if EXIT.load(Ordering::SeqCst) {
                break;
            }
        }
    }

    Ok(())
}

static PROJECT_DIRS: Lazy<ProjectDirs> = Lazy::new(|| ProjectDirs::from("", "", "m3").unwrap());

pub struct Config {
    doc: KdlDocument,
}

impl Config {
    fn new() -> eyre::Result<Self> {
        let cfg_dir = PROJECT_DIRS.config_local_dir();
        fs::create_dir_all(cfg_dir)?;
        let cfg_file = cfg_dir.join("m3.kdl");
        let mut cfg_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(cfg_file)?;
        let mut cfg_string = String::new();
        cfg_file.read_to_string(&mut cfg_string)?;
        let doc = cfg_string.parse()?;
        Ok(Self { doc })
    }

    fn installs(&self) -> impl Iterator<Item = Install> + '_ {
        self.doc
            .nodes()
            .iter()
            .filter(|node| node.name().value() == "install")
            .map(Install::new)
    }
}

struct Install<'a> {
    node: &'a KdlNode,
}

impl<'a> Install<'a> {
    fn new(node: &'a KdlNode) -> Self {
        Self { node }
    }

    fn target(&self) -> &Path {
        Path::new(
            self.node
                .children()
                .unwrap()
                .get("target")
                .unwrap()
                .get(0)
                .unwrap()
                .value()
                .as_string()
                .unwrap(),
        )
    }

    fn mounts(&self) -> impl Iterator<Item = Mount<'a>> {
        self.node
            .children()
            .unwrap()
            .nodes()
            .iter()
            .filter(|node| node.name().value() == "mount")
            .map(|node| Mount {
                node,
                at: node
                    .get("at")
                    .and_then(|at| at.value().as_string())
                    .map(Path::new),
                parent: node
                    .get("parent")
                    .and_then(|at| at.value().as_string())
                    .map(Path::new),
            })
    }
}

struct Mount<'a> {
    node: &'a KdlNode,
    at: Option<&'a Path>,
    parent: Option<&'a Path>,
}

impl<'a> Mount<'a> {
    fn at(&self) -> Option<&Path> {
        self.at
    }

    #[allow(dead_code)]
    fn parent(&self) -> Option<&Path> {
        self.parent
    }

    fn dirs<'b>(&'b self) -> impl Iterator<Item = MountDir<'a, 'b>> {
        self.node
            .children()
            .unwrap()
            .nodes()
            .iter()
            .filter(|node| node.name().value() == "dir")
            .map(|node| MountDir { node, mount: self })
    }
}

struct MountDir<'a, 'b> {
    node: &'a KdlNode,
    mount: &'b Mount<'a>,
}

impl<'a, 'b> MountDir<'a, 'b> {
    fn dir(&self) -> Cow<'a, Path> {
        let dir = self.node.get(0).unwrap().value().as_string().unwrap();
        self.mount
            .parent
            .map(|parent| parent.join(dir).into())
            .unwrap_or(Path::new(dir).into())
    }

    /// If this returns `Some`, then we only expose the enumerated files.
    fn files(&self) -> Option<impl Iterator<Item = &str>> {
        self.node
            .children()
            .map(|children| children.nodes().iter().map(|node| node.name().value()))
    }
}
