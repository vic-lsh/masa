use std::{env, path::PathBuf};

fn main() {
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("frontend_descriptor.bin"))
        .compile(&["proto/frontend.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("search_descriptor.bin"))
        .compile(&["proto/search.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("geo_descriptor.bin"))
        .compile(&["proto/geo.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("rate_descriptor.bin"))
        .compile(&["proto/rate.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("profile_descriptor.bin"))
        .compile(&["proto/profile.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("reservation_descriptor.bin"))
        .compile(&["proto/reservation.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("user_descriptor.bin"))
        .compile(&["proto/user.proto"], &["proto"])
        .unwrap();

    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("recommendation_descriptor.bin"))
        .compile(&["proto/recommendation.proto"], &["proto"])
        .unwrap();
    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("review_descriptor.bin"))
        .type_attribute(".", "#[derive(serde::Deserialize, serde::Serialize)]")
        .compile(&["proto/review.proto"], &["proto"])
        .unwrap();
}
