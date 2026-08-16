// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Shared runtime path computation for the daemon and client.
//!
//! Both processes must agree on where the command socket and lock file live.
//! Keeping the path logic in one place prevents them from drifting.

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use crate::AppResult;

/// Returns the XDG runtime directory for the current user.
///
/// This is used to locate the Unix socket and lock file. It is a strict
/// requirement that `XDG_RUNTIME_DIR` be set; falling back to `/tmp` would
/// allow other local users to intercept or control the daemon.
///
/// This function only *locates* the directory. The daemon additionally calls
/// [`validate_runtime_dir`] to enforce ownership and permissions before
/// trusting it.
pub fn runtime_dir() -> AppResult<PathBuf> {
    match std::env::var("XDG_RUNTIME_DIR") {
        Ok(dir) if !dir.is_empty() => Ok(PathBuf::from(dir)),
        _ => Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "XDG_RUNTIME_DIR environment variable is not set or is empty. \
             This is required for secure operation.",
        )
        .into()),
    }
}

/// Validates that the runtime directory is safe to bind a command socket in.
///
/// Fails closed: returns an error if the directory is missing, is not a
/// directory, is not owned by the effective user, or is accessible by
/// group/other. The daemon refuses to start rather than trust an unsafe
/// directory.
pub fn validate_runtime_dir() -> AppResult<PathBuf> {
    let dir = runtime_dir()?;
    // SAFETY: geteuid() never fails and takes no arguments.
    let euid = unsafe { libc::geteuid() };
    validate_dir(&dir, euid)?;
    Ok(dir)
}

/// Pure validation of a runtime directory against the expected euid.
///
/// `metadata` follows symlinks (the kernel checks the target's perms at
/// connect time too). Group/other access bits are rejected outright.
fn validate_dir(dir: &Path, euid: u32) -> std::io::Result<()> {
    let md = std::fs::metadata(dir)?; // fails if missing
    if !md.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotADirectory,
            format!("XDG_RUNTIME_DIR {} is not a directory", dir.display()),
        ));
    }
    if md.uid() != euid {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "XDG_RUNTIME_DIR {} is owned by uid {}, expected {euid}; \
                 do not run the daemon as root",
                dir.display(),
                md.uid()
            ),
        ));
    }
    if md.mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!(
                "XDG_RUNTIME_DIR {} is accessible by group/other (mode {:o}); refusing",
                dir.display(),
                md.mode() & 0o777
            ),
        ));
    }
    Ok(())
}

/// Path to the daemon's command socket.
pub fn socket_path() -> AppResult<PathBuf> {
    Ok(socket_path_in(&runtime_dir()?))
}

/// Socket path inside an arbitrary runtime directory (pure).
fn socket_path_in(runtime: &Path) -> PathBuf {
    runtime.join("eqtui").join("eqtui.sock")
}

/// Path to the daemon's lock file.
pub fn lock_path() -> AppResult<PathBuf> {
    Ok(lock_path_in(&runtime_dir()?))
}

/// Lock file path inside an arbitrary runtime directory (pure).
fn lock_path_in(runtime: &Path) -> PathBuf {
    runtime.join("eqtui").join("eqtui.lock")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Creates an owned temp dir with the given mode, returning its path.
    fn temp_dir(tag: &str, mode: u32) -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "eqtui-paths-{}-{}-{tag}",
            std::process::id(),
            tag.len()
        ));
        let dir = base;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(mode)).unwrap();
        dir
    }

    fn cleanup(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn owned_private_dir_is_valid() {
        // Arrange
        let euid = unsafe { libc::geteuid() };
        let dir = temp_dir("owned-0700", 0o700);

        // Act
        let result = validate_dir(&dir, euid);

        // Assert
        assert!(
            result.is_ok(),
            "0700 dir owned by euid should validate: {result:?}"
        );
        cleanup(&dir);
    }

    #[test]
    fn group_accessible_dir_is_rejected() {
        // Arrange
        let euid = unsafe { libc::geteuid() };
        let dir = temp_dir("group-0755", 0o755);

        // Act
        let result = validate_dir(&dir, euid);

        // Assert
        assert!(result.is_err(), "0755 dir must be rejected");
        cleanup(&dir);
    }

    #[test]
    fn missing_dir_is_rejected() {
        // Arrange
        let euid = unsafe { libc::geteuid() };
        let dir = std::env::temp_dir().join(format!("eqtui-paths-missing-{}", std::process::id()));

        // Act
        let result = validate_dir(&dir, euid);

        // Assert
        assert!(result.is_err(), "missing dir must be rejected");
    }

    #[test]
    fn regular_file_is_rejected() {
        // Arrange
        let euid = unsafe { libc::geteuid() };
        let file = std::env::temp_dir().join(format!("eqtui-paths-file-{}", std::process::id()));
        std::fs::write(&file, b"not a dir").unwrap();

        // Act
        let result = validate_dir(&file, euid);

        // Assert
        assert!(result.is_err(), "a regular file must be rejected");
        let _ = std::fs::remove_file(&file);
    }

    #[test]
    fn wrong_owner_is_rejected_when_root() {
        // Only meaningful as root: an owned-by-root dir owned by someone else.
        if unsafe { libc::geteuid() } != 0 {
            return;
        }

        // Arrange
        let dir = temp_dir("foreign-0700", 0o700);

        // Act: expect any non-zero uid to fail against euid 0's directory.
        let result = validate_dir(&dir, 1);

        // Assert
        assert!(
            result.is_err(),
            "dir not owned by expected uid must be rejected"
        );
        cleanup(&dir);
    }

    #[test]
    fn socket_path_uses_private_subdir() {
        // Arrange
        let fake_runtime = Path::new("/tmp/fake-runtime");

        // Act
        let path = socket_path_in(fake_runtime);

        // Assert
        assert_eq!(path, Path::new("/tmp/fake-runtime/eqtui/eqtui.sock"));
    }

    #[test]
    fn lock_path_uses_private_subdir() {
        // Arrange
        let fake_runtime = Path::new("/tmp/fake-runtime");

        // Act
        let path = lock_path_in(fake_runtime);

        // Assert
        assert_eq!(path, Path::new("/tmp/fake-runtime/eqtui/eqtui.lock"));
    }
}
