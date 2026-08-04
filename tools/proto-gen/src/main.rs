//! Code generation tool for compiling `renderd.proto` to `prost` Rust types.

use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path()?);

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let root_dir = manifest_dir.parent().unwrap().parent().unwrap();

    let proto_file = root_dir.join("proto").join("renderd.proto");
    let out_dir = root_dir
        .join("crates")
        .join("renderd-proto")
        .join("src")
        .join("generated");

    std::fs::create_dir_all(&out_dir)?;

    let mut config = prost_build::Config::new();
    config.out_dir(&out_dir);
    config.compile_protos(&[proto_file], &[root_dir.join("proto")])?;

    println!(
        "Successfully generated prost types in {}",
        out_dir.display()
    );
    Ok(())
}
