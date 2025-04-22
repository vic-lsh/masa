#![allow(dead_code)]

use log::error;
use std::sync::Arc;
use std::collections::HashMap;
use regex::Regex;


use tonic::{transport::Server, Request, Response, Status};

use text_svc::text_service::text_service_server::{TextService, TextServiceServer};
use text_svc::text_service::{TextReply, TextRequest};
use text_svc::user_mention_service::{user_mention_service_client::UserMentionServiceClient, 
                                        ComposeUserMentionRequest};

use text_svc::url_shorten_service::{url_shorten_service_client::UrlShortenServiceClient, 
                                        ComposeUrlsRequest};

pub mod text_svc {
    pub mod text_service {
        tonic::include_proto!("textservice");
    }
    pub mod user_mention_service {
        tonic::include_proto!("usermention");
    }
    pub mod url_shorten_service {
        tonic::include_proto!("url_shorten");
    }
}

#[derive(Debug)]
pub struct TextSvcImpl {
    url_shorten_client: UrlShortenServiceClient<tonic::transport::Channel>,
    user_mention_client: UserMentionServiceClient<tonic::transport::Channel>,
}

impl TextSvcImpl {
    pub fn new(
        url_shorten_client: UrlShortenServiceClient<tonic::transport::Channel>,
        user_mention_client: UserMentionServiceClient<tonic::transport::Channel>,
    ) -> Self {
        TextSvcImpl {
            url_shorten_client,
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
            let mut url_client_pool = self.url_shorten_client.clone();
            let url_links = &url_links;
            async move {
                let url_shorten_request = ComposeUrlsRequest {
                    req_id: 12345,
                    urls: url_links.clone(),
                };
                let response = url_client_pool
                    .compose_urls(Request::new(url_shorten_request))
                    .await;
                match response {
                    Ok(res) => {
                        let inner = res.into_inner();
                        if let Some(exception) = inner.exception {
                            return Err(Status::internal(exception.message));
                        }
                        return Ok(inner.urls);
                    }
                    Err(status) => {
                        error!("Error calling url_shorten service: {}", status);
                        return Err(status);
                    }
                }
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
        let Ok(result_urls) = shortened_url_future.await else {
            return Err(Status::internal("Failed to get shortened urls"));
        };

        let Ok(user_mentions) = user_mention_future.await else {
            return Err(Status::internal("Failed to get user mentions"));
        };

        println!("Shortened URLs: {:?}", result_urls);        
        println!("User mentions found: {:?}", user_mentions.len());

        let mut updated_text = text.clone();
        let mut shortened_urls: Vec<String> = Vec::new();
        for url in &result_urls {
            updated_text = updated_text.replace(&url.expanded_url, &url.shortened_url);
            shortened_urls.push(url.shortened_url.clone());
        }

        let mut user_mention_id: Vec<String> = Vec::new();
        for mention in &user_mentions {
            println!("User mention: {:?}", mention);
            let username = &mention.username;
            let user_id = mention.user_id;
            user_mention_id.push(user_id.to_string());
            updated_text = updated_text.replace(username, &format!("@{}", user_id));
        }

        let reply = TextReply {
            user_mentions: user_mention_id,
            urls: shortened_urls,
            updated_text: updated_text,
        };

        // return the processed text
        Ok(Response::new(reply))
    }
}

pub async fn create_service() -> TextServiceServer<TextSvcImpl> {
    let service = TextSvcImpl::new(
        UrlShortenServiceClient::connect("http://[::1]:50053").await.unwrap(),
        UserMentionServiceClient::connect("http://[::1]:50052").await.unwrap(),
    );
    TextServiceServer::new(service)
}
