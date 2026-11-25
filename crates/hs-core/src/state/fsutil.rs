//! Filesystem helpers with safe defaults.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Create a directory (and parents) if it does not exist.
///
/// # Errors
/// Propagates I/O errors.
pub fn ensure_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

/// Read a UTF-8 text file.
///
/// # Errors
/// Propagates I/O and UTF-8 errors.
pub fn read_to_string(path: &Path) -> std::io::Result<String> {
    fs::read_to_string(path)
}

/// Write a file atomically with `0600` permissions on Unix.
///
/// The content is written to a sibling temporary file, flushed to disk and then
/// renamed over the destination. On Windows the destination is removed first
/// because `rename` does not replace existing files there; this is only used for
/// development on Windows.
///
/// # Errors
/// Propagates I/O errors.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        let mut file = options.open(&tmp)?;

        #[cfg(unix)]
        {
            file.set_permissions(fs::Permissions::from_mode(0o600))?;
        }

        file.write_all(bytes)?;
        file.sync_all()?;
    }

    #[cfg(windows)]
    if path.exists() {
        fs::remove_file(path)?;
    }

    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_and_reads_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.json");
        write_private(&path, b"{\"ok\":true}").unwrap();
        assert_eq!(read_to_string(&path).unwrap(), "{\"ok\":true}");

        #[cfg(unix)]
        {
            let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.bin");
        write_private(&path, b"first").unwrap();
        write_private(&path, b"second").unwrap();
        assert_eq!(read_to_string(&path).unwrap(), "second");
    }
}
