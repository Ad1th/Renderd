//! Command lines for the child processes.

use crate::settings::{DisplayMode, HostSettings, ViewerSettings};

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

/// Arguments for `renderd-viewer` that match `settings`.
///
/// With `host` the viewer dials that address directly. Without it the viewer
/// looks for a host on the local network by itself.
#[must_use]
pub fn viewer_args(settings: &ViewerSettings, host: Option<&str>) -> Vec<String> {
    let mut args = vec![
        "--log-format".to_string(),
        "json".to_string(),
        "--codec".to_string(),
        settings.codec.as_arg().to_string(),
    ];
    if let Some(addr) = host {
        args.push("--host".to_string());
        args.push(addr.to_string());
    }
    if settings.fullscreen {
        args.push("--fullscreen".to_string());
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Codec;

    #[test]
    fn default_host_args_extend_on_the_standard_port() {
        assert_eq!(
            host_args(&HostSettings::default()),
            ["--log-format", "json", "--port", "4433", "--mode", "extend"]
        );
    }
}
