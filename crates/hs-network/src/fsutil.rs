//! Small filesystem helpers shared by the privileged `apply_*` functions.
//!
//! The appliance only ever writes a handful of generated configuration files
//! (nftables ruleset, netplan YAML). They are replaced through a temporary file
//! plus `rename` so that a crash mid-write never leaves a truncated ruleset on
//! disk (which could compromise the DMZ isolation on the next boot).

use std::fs;
use std::io::Write;
use std::path::Path;

use crate::error::{NetworkError, NetworkResult};

/// Write `contents` to `path`, replacing any previous file atomically.
///
/// The file is first written to a sibling temporary file and flushed to disk,
/// then renamed over the destination. On Windows `rename` refuses to replace an
/// existing file, so the destination is removed first (best effort, documented
/// degradation: the replacement is not atomic there).
///
/// # Errors
/// Returns [`NetworkError::Io`] when the parent directory cannot be created,
/// the temporary file cannot be written, or the rename fails.
pub(crate) fn atomic_write(path: &Path, contents: &str) -> NetworkResult<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| NetworkError::Invalid("target path has no file name".to_owned()))?;
    let temp_path = path.with_file_name(format!(".{file_name}.tmp"));

    {
        let mut file = fs::File::create(&temp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o644))?;
    }

    #[cfg(windows)]
    {
        if path.exists() {
            fs::remove_file(path)?;
        }
    }

    fs::rename(&temp_path, path).map_err(|err| {
        // Best effort cleanup; the error below is the actionable one.
        let _ = fs::remove_file(&temp_path);
        NetworkError::from(err)
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::atomic_write;

    #[test]
    fn writes_and_replaces_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("generated").join("rules.nft");
        atomic_write(&path, "first\n").expect("write");
        assert_eq!(fs::read_to_string(&path).expect("read"), "first\n");
        atomic_write(&path, "second\n").expect("replace");
        assert_eq!(fs::read_to_string(&path).expect("read"), "second\n");
        assert!(!dir.path().join("generated").join(".rules.nft.tmp").exists());
    }
}
