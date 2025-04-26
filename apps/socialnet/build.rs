fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::compile_protos("proto/uniqueid.proto")?;
    tonic_build::compile_protos("proto/media.proto")?;
    tonic_build::compile_protos("proto/url_shorten.proto")?;
    Ok(())
}
