//! Deciding what to run when the user presses start.

use crate::address::normalize_host;
use crate::command::{host_args, viewer_args};
use crate::role::Role;
use crate::settings::Settings;

/// What to start, worked out from the settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launch {
    /// Which binary to run.
    pub role: Role,
    /// Its arguments.
    pub args: Vec<String>,
    /// Viewer only: the address to dial, already normalised. `None` lets the
    /// viewer find a host on the network itself.
    pub host: Option<String>,
}

/// Plans a launch for the role in `settings`.
///
/// `typed_host` is what the user typed into the address field, if anything.
/// It only matters when viewing; blank text means "find the host for me".
///
/// # Errors
/// Returns a sentence for the window when the typed address is not usable.
pub fn plan(settings: &Settings, typed_host: Option<&str>) -> Result<Launch, String> {
    match settings.role {
        Role::Host => Ok(Launch {
            role: Role::Host,
            args: host_args(&settings.host),
            host: None,
        }),
        Role::Viewer => {
            let host = typed_host
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(normalize_host)
                .transpose()?;
            Ok(Launch {
                role: Role::Viewer,
                args: viewer_args(&settings.viewer, host.as_deref()),
                host,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(role: Role) -> Settings {
        Settings {
            role,
            ..Settings::default()
        }
    }

    #[test]
    fn sharing_ignores_a_typed_address() {
        let launch = plan(&settings(Role::Host), Some("10.0.0.1")).unwrap();
        assert_eq!(launch.role, Role::Host);
        assert_eq!(launch.host, None);
        assert!(!launch.args.contains(&"--host".to_string()));
    }

    #[test]
    fn viewing_dials_a_typed_address() {
        let launch = plan(&settings(Role::Viewer), Some(" 10.0.0.1 ")).unwrap();
        assert_eq!(launch.host.as_deref(), Some("10.0.0.1:4433"));
        assert!(launch.args.windows(2).any(|w| w == ["--host", "10.0.0.1:4433"]));
    }

    #[test]
    fn a_blank_address_means_discovery() {
        for typed in [None, Some(""), Some("   ")] {
            let launch = plan(&settings(Role::Viewer), typed).unwrap();
            assert_eq!(launch.host, None);
            assert!(!launch.args.contains(&"--host".to_string()));
        }
    }
}
