//! Command lines for the child processes.

use crate::settings::{DisplayMode, HostSettings};

/// Arguments for `renderd-host` that match `settings`.
///
/// JSON logs are always requested: they are how the app follows the session.
#[must_use]
pub fn host_args(settings: &HostSettings) -> Vec<String> {
    let mut args = vec![
        "--log-format".to_string(),
        "json".to_string(),
        "--port".to_string(),
        settings.port.to_string(),
        "--mode".to_string(),
        settings.mode.as_arg().to_string(),
    ];
    if settings.mode == DisplayMode::Mirror {
        args.push("--display-id".to_string());
        args.push(settings.display_id.to_string());
    }
    if settings.smooth {
        args.push("--smooth".to_string());
    }
    args
}
