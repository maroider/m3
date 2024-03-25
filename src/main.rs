use color_eyre::eyre;

mod fuse_fs;
mod vfs;

pub fn main() -> eyre::Result<()> {
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    fuse_fs::launch()?;

    Ok(())
}
