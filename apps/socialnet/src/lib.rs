pub mod media {
    tonic::include_proto!("media");
    pub mod client;
    pub mod server;
}

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
    pub mod client;
    pub mod db;
    pub mod server;
}

pub mod user_mention {
    tonic::include_proto!("usermention");

    pub mod client;
    pub mod db;
    pub mod server;
}

pub mod text_service {
    tonic::include_proto!("textservice");
    
    pub mod client;
    pub mod server;
}