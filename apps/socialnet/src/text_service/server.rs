#![allow(dead_code)]

use log::error;
use std::sync::Arc;
use std::collections::HashMap;
use regex::Regex;


use tonic::{transport::Server, Request, Response, Status};

use text_svc::text_service::text_service_server::{TextService, TextServiceServer};
use text_svc::text_service::{TextReply, TextRequest};
use text_svc::user_mention_service::{user_mention_service_client::UserMentionServiceClient, 
                                        ComposeUserMentionRequest, UserMention};

pub mod text_svc {
    pub mod text_service {
        tonic::include_proto!("textservice");
    }
    pub mod user_mention_service {
        tonic::include_proto!("usermention");
    }
}

#[derive(Debug)]
pub struct TextSvcImpl {
    url_client_pool: Arc<HashMap<&'static str, &'static str>>,
    user_mention_client: UserMentionServiceClient<tonic::transport::Channel>,
}

impl TextSvcImpl {
    pub fn new(
        url_client_pool: Arc<HashMap<&'static str, &'static str>>,
        user_mention_client: UserMentionServiceClient<tonic::transport::Channel>,
    ) -> Self {
        TextSvcImpl {
            url_client_pool,
            user_mention_client,
        }
    }
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
            let username = &word.as_str()[1..];
            mention_usernames.push(username.to_string());
        }

        // print the mentions
        println!("Mentioned usernames: {:?}", mention_usernames);

        // regx match url links with http or https
        let mut url_links = Vec::new();
        let re = Regex::new(r"(http://|https://)([a-zA-Z0-9_!~*'().&=+$%-]+)").unwrap();
        for word in re.find_iter(&text) {
            let url = &word.as_str();
            url_links.push(url.to_string());
        }

        // print the urls
        println!("URLs found: {:?}", url_links);

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
            let mut user_mention_client = self.user_mention_client.clone();
            let mention_usernames = &mention_usernames;
            async move {
                let user_mention_request = ComposeUserMentionRequest {
                    req_id: 12345,
                    usernames: mention_usernames.clone(),
                };
                let response = user_mention_client
                    .compose_user_mentions(Request::new(user_mention_request))
                    .await;
                match response {
                    Ok(res) => {
                        let inner = res.into_inner();
                        if let Some(exception) = inner.exception {
                            return Err(Status::internal(exception.message));
                        }
                        return Ok(inner.user_mentions);
                    }
                    Err(status) => {
                        error!("Error calling user_mention service: {}", status);
                        return Err(status);
                    }
                }           
            }
        };


        // process the text with url
        let shortened_urls = shortened_url_future.await;
        let Ok(user_mentions) = user_mention_future.await else {
            return Err(Status::internal("Failed to get user mentions"));
        };
        // print lenth of user mentions
        println!("User mentions found: {:?}", user_mentions.len());

        let mut updated_text = text.clone();
        for (url, shortened_url) in url_links.iter().zip(shortened_urls.iter()) {
            updated_text = updated_text.replace(url, shortened_url);
        }

        let mut user_mention_id: Vec<String> = Vec::new();
        for mention in &user_mentions {
            println!("User mention: {:?}", mention);
            let username = &mention.username;
            let user_id = mention.user_id;
            user_mention_id.push(user_id.to_string());
            updated_text = updated_text.replace(username, &format!("@{}[{}]", username, user_id));
        }

        let reply = TextReply {
            user_mentions: user_mention_id,
            urls: shortened_urls.into_iter().map(String::from).collect(),
            updated_text: updated_text.into(),
        };

        // return the processed text
        Ok(Response::new(reply))
    }
}



fn get_url_map() -> HashMap<&'static str, &'static str> {
    let mut map = HashMap::new();
    map.insert("https://openai.com", "http://s.io/gpt4");
    map.insert("https://www.example.com", "http://short.ly/abc123");
    map.insert("https://news.ycombinator.com", "http://hnr.cc/39572710");
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

pub async fn create_service() -> TextServiceServer<TextSvcImpl> {
    let service = TextSvcImpl::new(
        Arc::new(get_url_map()),
        UserMentionServiceClient::connect("http://[::1]:50052").await.unwrap(),
    );
    TextServiceServer::new(service)
}
