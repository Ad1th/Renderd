//! Integration tests validating default TOML configuration templates.

use std::path::PathBuf;

use renderd_config::{ConfigBuilder, ValidateConfig};

#[test]
fn test_host_default_template_validates() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();
    let host_template = root_dir.join("templates").join("renderd-host.default.toml");

    assert!(
        host_template.exists(),
        "Host template missing at {}",
        host_template.display()
    );

    let config = ConfigBuilder::new()
        .add_file(&host_template)
        .build()
        .expect("Failed to parse host default template");

    config
        .validate()
        .expect("Host default template failed validation rules");
}

#[test]
fn test_viewer_default_template_validates() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();
    let viewer_template = root_dir
        .join("templates")
        .join("renderd-viewer.default.toml");

    assert!(
        viewer_template.exists(),
        "Viewer template missing at {}",
        viewer_template.display()
    );

    let config = ConfigBuilder::new()
        .add_file(&viewer_template)
        .build()
        .expect("Failed to parse viewer default template");

    assert_eq!(config.viewer.window_width, 1920);
    assert_eq!(config.viewer.window_height, 1080);
    assert!(config.viewer.vsync);
}
