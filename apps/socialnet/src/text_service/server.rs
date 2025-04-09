#![allow(dead_code)]

use log::error;
use std::sync::Arc;
use std::collections::HashMap;
use regex::Regex;


use tonic::{transport::Server, Request, Response, Status};

use text_service::text_service_server::{TextService, TextServiceServer};
use text_service::{TextReply, TextRequest};

pub mod text_service {
    tonic::include_proto!("textservice");
}

#[derive(Debug, Default)]
pub struct TextSvcImpl {
    url_client_pool: Arc<HashMap<&'static str, &'static str>>,
    user_mention_client_pool: Arc<HashMap<&'static str, &'static str>>,
}

#[tonic::async_trait]
impl TextService for TextSvcImpl {
    async fn compose_text(
        &self,
        request: Request<TextRequest>,
    ) -> Result<Response<TextReply>, Status> {
        println!("Got a request: {:?}", request);

        let text:String = request.into_inner().text;
        
        // regx match mentions with @
        let mut mention_usernames = Vec::new();
        let re = Regex::new(r"@[a-zA-Z0-9-_]+").unwrap();
        for word in re.find_iter(&text) {
            let username = &word.as_str();
            mention_usernames.push(username.to_string());
        }

        // regx match url links with http or https
        let mut url_links = Vec::new();
        let re = Regex::new(r"(http://|https://)([a-zA-Z0-9_!~*'().&=+$%-]+)").unwrap();
        for word in re.find_iter(&text) {
            let url = &word.as_str();
            url_links.push(url.to_string());
        }

        // async func to get shortened url
        let shortened_url_future = {
            let url_client_pool = &self.url_client_pool;
            let url_links = &url_links;
            async move {
                let mut shortened_urls = Vec::new();
                for url in url_links {
                    match url_client_pool.get(&url as &str) {
                        Some(shortened_url) => {
                            shortened_urls.push(shortened_url.to_string());
                        }
                        None => {
                            error!("URL not found for: {}", url);
                            continue;
                        }
                    }
                }
                shortened_urls
            }
        };

        // async func to get user mention、
        let user_mention_future = {
            let user_client_pool = &self.user_mention_client_pool;
            let mention_usernames = &mention_usernames;
            async move {
                let mut user_mentions = Vec::new();
                for username in mention_usernames {
                    match user_client_pool.get(&username as &str) {
                        Some(user_id) => {
                            user_mentions.push(user_id.to_string());
                        }
                        None => {
                            error!("User not found for: {}", username);
                            continue;
                        }
                    }
                }
                user_mentions
            }
        };


        // process the text with url
        let user_mentions = user_mention_future.await;
        let shortened_urls = shortened_url_future.await;

        let mut updated_text = text.clone();
        for (url, shortened_url) in url_links.iter().zip(shortened_urls.iter()) {
            updated_text = updated_text.replace(url, shortened_url);
        }

        let reply = TextReply {
            user_mentions: user_mentions.into_iter().map(String::from).collect(),
            urls: shortened_urls.into_iter().map(String::from).collect(),
            updated_text: updated_text.into(),
        };

        // return the processed text
        Ok(Response::new(reply))
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "[::1]:50051".parse()?;
    let url_map = Arc::new(get_url_map());
    let user_map = Arc::new(get_user_map());
    let textsvc = TextSvcImpl {
        url_client_pool: Arc::clone(&url_map),
        user_mention_client_pool: Arc::clone(&user_map),
    };

    Server::builder()
        .add_service(TextServiceServer::new(textsvc))
        .serve(addr)
        .await?;

    Ok(())
}

fn get_url_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("https://openai.com/research/gpt-4", "http://s.io/gpt4");
    map.insert("https://www.example.com/articles/rust-tokio", "http://short.ly/abc123");
    map.insert("https://news.ycombinator.com/item?id=39572710", "http://hnr.cc/39572710");
    map
}

fn get_user_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("vic", "001");
    map.insert("michael", "002");
    map.insert("jasper", "003");
    map.insert("baris", "004");
    map.insert("ratul", "005");
    map
}


