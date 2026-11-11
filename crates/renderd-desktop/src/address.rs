//! Host addresses typed by a person.

use std::net::{IpAddr, SocketAddr};

use crate::settings::DEFAULT_PORT;

/// Turns what someone typed into the `ip:port` form the viewer takes.
///
/// Accepts `192.168.1.20`, `192.168.1.20:4433`, `::1` and `[::1]:4433`. A
/// missing port becomes the default one. Surrounding spaces are ignored.
///
/// # Errors
/// Returns a short sentence for the window when the text is not an address.
pub fn normalize_host(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Enter the address shown on the other computer.".to_string());
    }
    if let Ok(addr) = trimmed.parse::<SocketAddr>() {
        return Ok(addr.to_string());
    }
    if let Ok(ip) = trimmed.parse::<IpAddr>() {
        return Ok(SocketAddr::new(ip, DEFAULT_PORT).to_string());
    }
    Err(format!(
        "\"{trimmed}\" is not an address. It should look like 192.168.1.20 or 192.168.1.20:4433."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_ip_gets_the_default_port() {
        assert_eq!(normalize_host("192.168.1.20").unwrap(), "192.168.1.20:4433");
    }

    #[test]
    fn an_explicit_port_is_kept_and_spaces_trimmed() {
        assert_eq!(normalize_host("  10.0.0.7:5000 ").unwrap(), "10.0.0.7:5000");
    }

    #[test]
    fn ipv6_addresses_are_accepted() {
        assert_eq!(normalize_host("::1").unwrap(), "[::1]:4433");
        assert_eq!(normalize_host("[::1]:9000").unwrap(), "[::1]:9000");
    }

    #[test]
    fn nonsense_is_rejected_with_a_hint() {
        let error = normalize_host("my-mac").unwrap_err();
        assert!(error.contains("my-mac"));
        assert!(error.contains("192.168.1.20"));
        assert!(normalize_host("   ").is_err());
    }
}
