//! Panic hook configuration for `renderd-host`.

/// Installs a custom panic hook that logs structured error details via `tracing` before process exit.
pub fn setup_panic_hook() {
    std::panic::set_hook(Box::new(|panic_info| {
        let payload = panic_info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| panic_info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "Unknown panic payload".to_string());

        let location = panic_info.location().map_or_else(
            || "unknown location".to_string(),
            |loc| format!("{}:{}", loc.file(), loc.line()),
        );

        tracing::error!(
            panic.payload = %payload,
            panic.location = %location,
            "Application panicked! Aborting process."
        );
    }));
}
