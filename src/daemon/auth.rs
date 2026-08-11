// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Peer authorization for accepted IPC connections.

use std::os::unix::net::UnixStream;

use tracing::warn;
use uds::UnixStreamExt;

/// Returns `true` if the connected peer belongs to the expected uid.
///
/// Uses `SO_PEERCRED` (via `uds`) so a swapped-out or misconfigured runtime
/// directory cannot open the command socket to other users. On failure to
/// read credentials the connection is rejected (fail closed).
pub(super) fn peer_is_self(stream: &UnixStream, euid: u32) -> bool {
    match stream.initial_peer_credentials() {
        Ok(cred) if cred.euid() == euid => true,
        Ok(cred) => {
            warn!(
                euid = cred.euid(),
                "Rejected IPC connection from foreign uid"
            );
            false
        }
        Err(e) => {
            warn!(%e, "Could not read peer credentials; rejecting");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A same-process socket pair reports the caller's own uid on Linux.
    #[test]
    fn peer_credentials_report_own_uid() {
        // Arrange
        let (a, b) = UnixStream::pair().expect("socketpair should succeed");
        let euid = unsafe { libc::geteuid() };

        // Act
        let cred_a = a.initial_peer_credentials().expect("peer creds readable");
        let cred_b = b.initial_peer_credentials().expect("peer creds readable");

        // Assert
        assert_eq!(cred_a.euid(), euid);
        assert_eq!(cred_b.euid(), euid);
    }

    /// `peer_is_self` accepts a same-uid peer.
    #[test]
    fn peer_is_self_accepts_same_user() {
        // Arrange
        let (a, _b) = UnixStream::pair().expect("socketpair should succeed");
        let euid = unsafe { libc::geteuid() };

        // Act
        let accepted = peer_is_self(&a, euid);

        // Assert
        assert!(accepted, "same-euid peer must be accepted");
    }

    /// `peer_is_self` rejects a peer whose uid differs from the expected one.
    #[test]
    fn peer_is_self_rejects_foreign_uid() {
        // Arrange
        let (a, _b) = UnixStream::pair().expect("socketpair should succeed");
        let euid = unsafe { libc::geteuid() };
        // Flip the lowest bit: always a different uid from our own.
        let foreign = euid ^ 1;

        // Act
        let accepted = peer_is_self(&a, foreign);

        // Assert
        assert!(!accepted, "peer with foreign uid must be rejected");
    }
}
