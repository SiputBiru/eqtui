// Copyright (C) 2026 SiputBiru <radityamahatma23@gmail.com>
// SPDX-License-Identifier: GPL-2.0-only

//! Per-client connection handling: bounded request framing and responses.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

use pipewire::channel;
use tracing::{debug, error, warn};

use crate::protocol::{Request, Response};

use super::dispatch::dispatch_request;
use super::state::DaemonState;

/// Serves one client connection: reads bounded JSON-lines requests, dispatches
/// them, and writes responses until the client disconnects.
pub(super) fn handle_client(
    stream: UnixStream,
    state: Arc<DaemonState>,
    cmd_tx: channel::Sender<crate::state::PwCommand>,
    client_id: u64,
) {
    let read_stream = match stream.try_clone() {
        Ok(s) => s,
        Err(e) => {
            error!(%e, client_id, "Failed to clone client stream for reading");
            return;
        }
    };

    state.register_client(&stream, client_id);

    // 64 KiB covers ~31 bands with generous headroom; anything larger is hostile.
    const MAX_REQUEST_LINE: u64 = 64 * 1024;

    let mut reader = BufReader::new(read_stream);
    loop {
        let mut buf = Vec::new();
        // `take` caps bytes read from the stream, so a missing newline can't
        // grow `buf` past the limit.
        let n = match reader
            .by_ref()
            .take(MAX_REQUEST_LINE + 1)
            .read_until(b'\n', &mut buf)
        {
            Ok(n) => n,
            Err(e) => {
                debug!(%e, client_id, "Read error; closing connection");
                break;
            }
        };
        if n == 0 {
            break; // client closed
        }
        if buf.len() as u64 > MAX_REQUEST_LINE || !buf.ends_with(b"\n") {
            let _ = send_resp(
                &stream,
                Response {
                    ok: false,
                    error: Some(format!("request line exceeds {MAX_REQUEST_LINE} bytes")),
                    status: None,
                },
            );
            warn!(client_id, "Oversized request line; disconnecting client");
            break;
        }

        let trimmed = String::from_utf8_lossy(&buf);
        let trimmed = trimmed.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Request = match serde_json::from_str(trimmed) {
            Ok(r) => r,
            Err(e) => {
                let _ = send_resp(
                    &stream,
                    Response {
                        ok: false,
                        error: Some(format!("Invalid JSON: {e}")),
                        status: None,
                    },
                );
                continue;
            }
        };

        let resp = dispatch_request(req, &state, &cmd_tx);
        let _ = send_resp(&stream, resp);
    }

    state.unregister_client(client_id);
}

fn send_resp(mut stream: &UnixStream, resp: Response) -> std::io::Result<()> {
    let json = serde_json::to_string(&resp)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    stream.write_all(json.as_bytes())?;
    stream.write_all(b"\n")?;
    Ok(())
}
