//! Captures a few frames of the main display and logs how they are tagged.
//! Needs Screen Recording permission for the terminal running it.
#![cfg(target_os = "macos")]

use std::time::Duration;

use renderd_sc_sys::{ContentFilter, ScreenStream};

fn main() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    let filter = ContentFilter::main_display().expect("main display");
    let stream =
        ScreenStream::with_dimensions(&filter, filter.width(), filter.height(), 30, |_| {})
            .expect("stream");
    stream.start().expect("start");
    std::thread::sleep(Duration::from_secs(2));
}
