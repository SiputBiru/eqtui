use std::process::Command;

fn run_pw_link(args: &[&str]) -> bool {
    let output = match Command::new("pw-link").args(args).output() {
        Ok(o) => o,
        Err(e) => {
            tracing::error!("failed to execute pw-link: {e}");
            return false;
        }
    };

    if output.status.success() {
        tracing::info!(args = ?args, "pw-link success");
        return true;
    }

    let stderr = String::from_utf8_lossy(&output.stderr);

    if stderr.contains("File exists") {
        tracing::debug!("pw-link link already exists (skipped)");
        return false;
    }
    if stderr.contains("No such file") {
        tracing::debug!("pw-link link not found (already removed)");
        return false;
    }

    tracing::error!("pw-link failed: {}", stderr.trim());
    false
}

pub(crate) fn create_monitor_links(out_node_id: u32, in_node_id: u32) {
    for (out_port, in_port) in &[("monitor_FL", "input_FL"), ("monitor_FR", "input_FR")] {
        run_pw_link(&[
            &format!("{out_node_id}:{out_port}"),
            &format!("{in_node_id}:{in_port}"),
        ]);
    }
}

/// Create `PipeWire` links from the DSP filter's output ports to a target
/// device's playback ports using `pw-link`.
pub(crate) fn create_device_output_links(filter_id: u32, device_id: u32) {
    for (out_port, in_port) in &[("output_FL", "playback_FL"), ("output_FR", "playback_FR")] {
        run_pw_link(&[
            &format!("{filter_id}:{out_port}"),
            &format!("{device_id}:{in_port}"),
        ]);
    }
}

/// Remove `PipeWire` links between the DSP filter's output ports and a
/// target device's playback ports using `pw-link -d`.
pub(crate) fn remove_device_output_links(filter_id: u32, device_id: u32) {
    for (out_port, in_port) in &[("output_FL", "playback_FL"), ("output_FR", "playback_FR")] {
        run_pw_link(&[
            "-d",
            &format!("{filter_id}:{out_port}"),
            &format!("{device_id}:{in_port}"),
        ]);
    }
}

/// Check whether any `PipeWire` link routes audio INTO the null sink's
/// `playback_FL` or `playback_FR` ports.  Returns `true` if at least one
/// audio source is connected to the null-sink input.
pub(crate) fn check_null_sink_input_source(null_sink_id: u32) -> bool {
    let Ok(output) = Command::new("pw-link").arg("-I").output() else {
        return false;
    };

    let text = String::from_utf8_lossy(&output.stdout);
    text.lines().any(|line| {
        line.contains(&format!("-> {null_sink_id}:playback_FL"))
            || line.contains(&format!("-> {null_sink_id}:playback_FR"))
    })
}
