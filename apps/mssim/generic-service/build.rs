fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use vendored protoc to avoid system dependency issues
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());

    tonic_build::compile_protos("proto/service.proto")?;
    Ok(())
}
