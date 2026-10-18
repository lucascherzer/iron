//! Linux/macOS implementations of iron's platform seams.
//! Android supplies its own app-private paths and secure storage.

pub mod dns_config;
mod tun;
pub use tun::DesktopTun;

use anyhow::{Context, Result};
use iron_core::platform::PrivateFileWriter;
use iron_core::{IrohConfig, NodeConfig, StatePaths};
use std::fs::{self, File};
use std::io::Write;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
compile_error!("iron-desktop supports Linux and macOS only");

#[cfg(target_os = "linux")]
const STATE_DIRECTORY: &str = "/var/lib/iron";
#[cfg(target_os = "macos")]
const STATE_DIRECTORY: &str = "/Library/Application Support/iron";
#[cfg(target_os = "linux")]
const CONFIG_FILE: &str = "/etc/iron/config.toml";
#[cfg(target_os = "macos")]
const CONFIG_FILE: &str = "/Library/Application Support/iron/config.toml";

/// Default system-owned persistent state.
pub fn default_state_paths() -> StatePaths {
    StatePaths::new(STATE_DIRECTORY)
}

/// Default system-wide configuration file.
pub fn default_config_file() -> PathBuf {
    PathBuf::from(CONFIG_FILE)
}

#[derive(Debug, Default)]
pub struct DesktopPrivateFileWriter;

impl PrivateFileWriter for DesktopPrivateFileWriter {
    fn create_new(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        publish(path, false, |file| file.write_all(bytes))
    }
}

impl DesktopPrivateFileWriter {
    /// Replace an existing private file after explicit caller authorization.
    pub fn replace(&self, path: &Path, bytes: &[u8]) -> Result<()> {
        publish(path, true, |file| file.write_all(bytes))
    }
}

fn publish(
    path: &Path,
    replace: bool,
    write: impl FnOnce(&mut File) -> std::io::Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .context("creating private state directory")?;

    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    // NamedTempFile is private (0600 on Unix) from the outset.
    write(temporary.as_file_mut())?;
    temporary.as_file().sync_all()?;
    if replace {
        temporary.persist(path).map_err(|error| error.error)?;
    } else {
        temporary
            .persist_noclobber(path)
            .map_err(|error| error.error)?;
    }
    sync_directory(parent)?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// Desktop defaults: system state, loopback DNS, and self-provisioned TUN.
pub fn desktop_node_config() -> NodeConfig {
    NodeConfig {
        paths: default_state_paths(),
        storage: Arc::new(DesktopPrivateFileWriter),
        iroh: IrohConfig::default(),
        dns_listen: SocketAddr::from(([127, 0, 0, 1], 5333)),
        dns_upstream: vec![],
        tun: Box::new(DesktopTun),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_paths_are_system_wide() {
        #[cfg(target_os = "linux")]
        let (directory, config) = ("/var/lib/iron", "/etc/iron/config.toml");
        #[cfg(target_os = "macos")]
        let (directory, config) = (
            "/Library/Application Support/iron",
            "/Library/Application Support/iron/config.toml",
        );
        let paths = default_state_paths();
        assert_eq!(paths.key_file(), Path::new(directory).join("secret.key"));
        assert_eq!(paths.state_file(), Path::new(directory).join("state.db"));
        assert_eq!(default_config_file(), PathBuf::from(config));
    }
    #[test]
    fn creation_is_private_and_never_replaces() {
        let temp = tempfile::tempdir().unwrap();
        let existing_parent = temp.path().join("shared");
        fs::create_dir(&existing_parent).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&existing_parent, fs::Permissions::from_mode(0o755)).unwrap();
        }
        DesktopPrivateFileWriter
            .create_new(&existing_parent.join("existing-parent-key"), b"private")
            .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&existing_parent).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        let path = temp.path().join("private/subdir/key");
        DesktopPrivateFileWriter
            .create_new(&path, b"first")
            .unwrap();
        assert!(
            DesktopPrivateFileWriter
                .create_new(&path, b"second")
                .is_err()
        );
        assert_eq!(fs::read(&path).unwrap(), b"first");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(path.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn authorized_replacement_is_private_and_failed_write_preserves_identity() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("key");
        DesktopPrivateFileWriter.create_new(&path, b"old").unwrap();
        DesktopPrivateFileWriter.replace(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let directory = temp.path().join("directory");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("identity"), b"keep").unwrap();
        assert!(
            DesktopPrivateFileWriter
                .replace(&directory, b"replacement")
                .is_err()
        );
        assert_eq!(fs::read(directory.join("identity")).unwrap(), b"keep");
    }

    #[test]
    fn failed_staging_write_never_publishes_partial_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("key");
        let result = publish(&path, false, |file| {
            file.write_all(b"part")?;
            Err(std::io::Error::other("simulated write failure"))
        });
        assert!(result.is_err());
        assert!(!path.exists());
    }
}
