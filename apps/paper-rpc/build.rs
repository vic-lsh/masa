fn main() {
    tonic_build::configure()
        .default_hooks_path("crate::PaperHooks")
        .compile(&["proto/paper.proto"], &["proto"])
        .expect("compile paper RPC service");
}
