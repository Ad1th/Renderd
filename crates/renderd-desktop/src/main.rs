//! The Renderd desktop app.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

/// The window's assets and configuration, embedded at build time.
// The generated code ends a failed asset lookup with `std::process::exit`,
// which the workspace bans in its own code.
#[allow(clippy::disallowed_methods)]
fn context() -> tauri::Context {
    tauri::generate_context!()
}

fn main() {
    tauri::Builder::default()
        .run(context())
        .expect("the Renderd window could not be created");
}
