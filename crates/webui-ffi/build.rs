extern crate cbindgen;

use std::env;
use std::path::PathBuf;

fn main() {
    let crate_dir = env::var("CARGO_MANIFEST_DIR").unwrap();
    let out_dir = PathBuf::from(crate_dir).join("include");

    // Create include directory if it doesn't exist
    std::fs::create_dir_all(&out_dir).unwrap();

    // C header
    let config = cbindgen::Config::default();
    cbindgen::Builder::new()
        .with_crate(crate_dir.clone())
        .with_config(config)
        .generate()
        .expect("Unable to generate C bindings")
        .write_to_file(out_dir.join("webui_ffi.h"));
}
