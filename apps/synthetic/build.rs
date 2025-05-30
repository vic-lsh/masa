use std::{env, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("frontend_descriptor.bin"))
        .compile(&["proto/frontend.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("child_descriptor.bin"))
        .compile(&["proto/child.proto"], &["proto"])
        .unwrap();
}
