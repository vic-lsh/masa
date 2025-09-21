fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    tonic_build::compile_protos("proto/social_graph.proto")?;
    Ok(())
}
