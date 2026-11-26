//! Logging policy.
//!
//! Logs go to the OS log directory (and stdout in debug builds) at `Info` (Pairpane's own `Debug` lines, such as
//! listing timings, too in debug builds). They describe what
//! the app does, never what the user does: file names, file contents, IPC payloads and secrets
//! must never be logged. The filter drops the targets that could carry payloads, and the webview
//! cannot log at all (the log plugin's `log` command is not granted).

use tauri::Runtime;
use tauri::plugin::TauriPlugin;
use tauri_plugin_log::log::{LevelFilter, Metadata};
use tauri_plugin_log::{RotationStrategy, Target, TargetKind, WEBVIEW_TARGET};

/// Log targets that may carry IPC payloads or webview console output.
const PAYLOAD_TARGETS: &[&str] = &["tauri::ipc", WEBVIEW_TARGET];

/// Crates under the device link (iroh, its mDNS lookup, and the QUIC, DNS, netlink and async crates it uses) that log
/// addresses of the user's devices and of the networks they are on, or their own state, and the SSH crates under the
/// remote sites (they log through `log` and can name hosts, users and key files): nothing of them is logged.
const NETWORK_TARGETS: &[&str] = &[
    "iroh",
    "iroh_base",
    "iroh_dns",
    "iroh_relay",
    "iroh_metrics",
    "iroh_mdns_address_lookup",
    "swarm_discovery",
    "noq",
    "noq_proto",
    "noq_udp",
    "netwatch",
    "netdev",
    "netlink_proto",
    "netlink_packet_core",
    "netlink_packet_route",
    "netlink_sys",
    "hickory_proto",
    "hickory_resolver",
    "n0_dns_resolver",
    "n0_watcher",
    "n0_future",
    "n0_error",
    "simple_dns",
    "tokio_websockets",
    "acto",
    "russh",
    "russh_sftp",
    "russh_cryptovec",
    "pageant",
];

/// Crates that the link shares with the updater and that log what a peer on the network sent while it is refused (the
/// TLS stack names the server name of a QUIC handshake in its warnings): only their errors are logged.
const ERRORS_ONLY_TARGETS: &[&str] = &["rustls"];

/// Size at which the log file is rotated.
const MAX_LOG_FILE_BYTES: u128 = 1024 * 1024;

/// Number of rotated log files kept.
const KEPT_LOG_FILES: usize = 3;

/// The configured log plugin.
pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    let mut targets = vec![Target::new(TargetKind::LogDir { file_name: None })];

    if cfg!(debug_assertions) {
        targets.push(Target::new(TargetKind::Stdout));
    }

    let own_level = if cfg!(debug_assertions) {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    let mut builder = tauri_plugin_log::Builder::new()
        .clear_targets()
        .targets(targets)
        .level(LevelFilter::Info)
        .level_for("pairpane_core", own_level)
        .level_for("pairpane_app_lib", own_level)
        // zbus logs every single-instance D-Bus call (including its argv signature) at info.
        .level_for("zbus", LevelFilter::Warn)
        // notify's own log lines can name the watched folder; its errors reach our handler, which logs a fixed reason.
        .level_for("notify", LevelFilter::Off)
        // The link's own lines are warnings about the service; the network stack under it logs addresses of the user's
        // devices and of the networks they are on (iroh, its mDNS lookup and the QUIC and network crates it uses).
        .level_for("pairpane_link", LevelFilter::Warn)
        // The same for the remote sites: its warnings are about the service, never about a host.
        .level_for("pairpane_remote", LevelFilter::Warn);

    for target in NETWORK_TARGETS {
        builder = builder.level_for(*target, LevelFilter::Off);
    }
    for target in ERRORS_ONLY_TARGETS {
        builder = builder.level_for(*target, LevelFilter::Error);
    }

    builder
        .filter(is_payload_free)
        .max_file_size(MAX_LOG_FILE_BYTES)
        .rotation_strategy(RotationStrategy::KeepSome(KEPT_LOG_FILES))
        .build()
}

fn is_payload_free(metadata: &Metadata) -> bool {
    let target = metadata.target();

    !PAYLOAD_TARGETS.iter().any(|blocked| target.starts_with(blocked))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_network_stack_under_the_link_is_silenced() {
        // The crates of the link's tree that log addresses, resolvers' answers or the state of the network.
        let required = [
            "iroh",
            "iroh_base",
            "iroh_dns",
            "iroh_relay",
            "iroh_metrics",
            "iroh_mdns_address_lookup",
            "swarm_discovery",
            "noq",
            "noq_proto",
            "noq_udp",
            "netwatch",
            "netlink_proto",
            "netlink_packet_core",
            "netlink_packet_route",
            "netlink_sys",
            "hickory_proto",
            "hickory_resolver",
            "n0_dns_resolver",
            "acto",
        ];
        // The SSH crates under the remote sites: every crate of their tree that logs (`cargo tree -p pairpane_remote`).
        let ssh = ["russh", "russh_sftp", "russh_cryptovec", "pageant"];

        for target in required.into_iter().chain(ssh) {
            assert!(NETWORK_TARGETS.contains(&target), "{target} is not silenced");
        }
        // Any host on the network can start a QUIC handshake with the link and so make the TLS stack warn.
        assert!(ERRORS_ONLY_TARGETS.contains(&"rustls"), "rustls warnings are logged");
    }

    #[test]
    fn payload_targets_are_dropped_and_the_rest_pass() {
        let blocked = Metadata::builder().target("tauri::ipc::channel").build();
        let console = Metadata::builder().target(WEBVIEW_TARGET).build();
        let own = Metadata::builder().target("pairpane_core::fs").build();

        assert!(!is_payload_free(&blocked));
        assert!(!is_payload_free(&console));
        assert!(is_payload_free(&own));
    }
}
