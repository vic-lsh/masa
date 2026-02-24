fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Use vendored protoc to avoid system dependency issues
    std::env::set_var("PROTOC", protoc_bin_vendored::protoc_bin_path().unwrap());

    tonic_build::compile_protos("proto/textservice.proto")?;
    tonic_build::compile_protos("proto/media.proto")?;
    tonic_build::compile_protos("proto/url_shorten.proto")?;
    tonic_build::compile_protos("proto/usermention.proto")?;
    tonic_build::compile_protos("proto/user.proto")?;
    tonic_build::compile_protos("proto/user_timeline.proto")?;
    tonic_build::compile_protos("proto/home_timeline.proto")?;
    tonic_build::compile_protos("proto/post_storage.proto")?;
    tonic_build::compile_protos("proto/uniqueid.proto")?;
    tonic_build::compile_protos("proto/compose_post.proto")?;
    tonic_build::compile_protos("proto/register_user.proto")?;
    tonic_build::compile_protos("proto/frontend.proto")?;
    tonic_build::compile_protos("proto/social_graph.proto")?;
    Ok(())
}
