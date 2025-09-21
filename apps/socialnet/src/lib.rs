pub mod media {
    tonic::include_proto!("media");
    pub mod server;
}

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
    pub mod db;
    pub mod server;
}

pub mod user_mention {
    tonic::include_proto!("usermention");

    pub mod db;
    pub mod server;
}

pub use user_mention as usermention;

pub mod text_service {
    tonic::include_proto!("textservice");

    pub mod server;
}

pub mod user {
    tonic::include_proto!("user");

    pub mod server;
}

pub mod user_timeline {
    tonic::include_proto!("user_timeline");

    pub mod server;
}

pub mod post_storage {
    tonic::include_proto!("post_storage");

    pub use crate::media;
    pub use crate::url_shorten;
    pub use crate::user;
    pub use crate::user_mention as usermention;
    pub use crate::user_timeline;

    pub mod server;
}
