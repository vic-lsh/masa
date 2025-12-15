#![allow(dead_code)]

use log::error;
use regex::Regex;
use log::info;

use tonic::{Request, Response, Status};

use text_svc::text_service::text_service_server::{TextService, TextServiceServer};
use text_svc::text_service::{TextReply, TextRequest};
use text_svc::user_mention_service::{
    user_mention_service_client::UserMentionServiceClient, ComposeUserMentionRequest,
};

use text_svc::url_shorten_service::{
    url_shorten_service_client::UrlShortenServiceClient, ComposeUrlsRequest,
};

use std::env;
use tonic::transport::Channel;
use tonic::transport::masa_channel::LoadBalancedChannel;




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


#[derive(Clone, Debug)]
pub struct Args {
    pub url_shorten_service_ip: String,
    pub url_shorten_service_port: u16,
    pub url_shorten_service_replicas: u8,

    pub user_mention_service_ip: String,
    pub user_mention_service_port: u16,
    pub user_mention_service_replicas: u8,
}

#[derive(Clone)]
pub struct TextSvcImpl {
    url_shorten_client: UrlShortenServiceClient<LoadBalancedChannel>,
    user_mention_client: UserMentionServiceClient<LoadBalancedChannel>,
}

impl Args {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            url_shorten_service_ip: env::var("URL_SHORTEN_SERVICE_IP")
                .unwrap_or_else(|_| "socialnet-url-shorten-service".to_string()),
            url_shorten_service_port: env::var("URL_SHORTEN_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            url_shorten_service_replicas: env::var("URL_SHORTEN_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()?,

            user_mention_service_ip: env::var("USER_MENTION_SERVICE_IP")
                .unwrap_or_else(|_| "socialnet-user-mention-service".to_string()),
            user_mention_service_port: env::var("USER_MENTION_SERVICE_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse()?,
            user_mention_service_replicas: env::var("USER_MENTION_SERVICE_REPLICAS")
                .unwrap_or_else(|_| "1".to_string())
                .parse()?,
        })
    }
}

impl TextSvcImpl {
    pub async fn new(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        let url_shorten_channel = LoadBalancedChannel::new(
            args.url_shorten_service_ip.clone(),
            args.url_shorten_service_port,
            args.url_shorten_service_replicas,
        ).await;
        let url_shorten_client = UrlShortenServiceClient::new(url_shorten_channel);

        let user_mention_channel = LoadBalancedChannel::new(
            args.user_mention_service_ip.clone(),
            args.user_mention_service_port,
            args.user_mention_service_replicas,
        ).await;
        let user_mention_client = UserMentionServiceClient::new(user_mention_channel);

        Ok(TextSvcImpl {
            url_shorten_client,
            user_mention_client,
        })
    }
}

#[tonic::async_trait]
impl TextService for TextSvcImpl {
    async fn compose_text(
        &self,
        request: Request<TextRequest>,
    ) -> Result<Response<TextReply>, Status> {
        println!("Got a request: {:?}", request);

        let text: String = request.into_inner().text;

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
        let shortened_url_task = {
            let mut url_client_pool = self.url_shorten_client.clone();
            let url_links = url_links.clone();
            tokio::spawn(async move {
                // FIX: Check if empty before making the network call
                if url_links.is_empty() {
                    return Ok(vec![]); 
                }

                let url_shorten_request = ComposeUrlsRequest {
                    req_id: 12345,
                    urls: url_links,
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
                        // Use println! because your logger might not be initialized to stdout
                        println!("Error calling url_shorten service: {:?}", status); 
                        return Err(status);
                    }
                }
            })
        };


        // async func to get user mention、
        let user_mention_task = {
            let mut user_mention_client = self.user_mention_client.clone();
            let mention_usernames = mention_usernames.clone();
            tokio::spawn(async move {
                let user_mention_request = ComposeUserMentionRequest {
                    req_id: 12345,
                    usernames: mention_usernames,
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
            })
        };

        // process the text with url
        let Ok(result_urls) = shortened_url_task
            .await
            .expect("shortened url task shouldn't fail")
        else {
            return Err(Status::internal("Text Service: Failed to get shortened urls"));
        };

        let Ok(user_mentions) = user_mention_task
            .await
            .expect("user mention task shoudln't fail")
        else {
            return Err(Status::internal("Text Service: Failed to get user mentions"));
        };

        println!("Shortened URLs: {:?}", result_urls);

        let mut updated_text = text.clone();
        let mut shortened_urls: Vec<String> = Vec::new();
        for url in &result_urls {
            updated_text = updated_text.replace(&url.expanded_url, &url.shortened_url);
            shortened_urls.push(url.shortened_url.clone());
        }

        let mut user_mention_id: Vec<String> = Vec::new();
        for mention in &user_mentions {
            println!("User mention: {:?}", mention);
            let Some((username, _)) = mention.username.split_once('@') else {
                error!("Invalid user mention format: {}", mention.username);
                continue;
            };
            let user_id = mention.user_id;
            user_mention_id.push(user_id.to_string());
            updated_text =
                updated_text.replace(&format!("@{}", username), &format!("user_id:{}", user_id));
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

    println!("trying to connect text service");
    let args = Args::from_env().expect("Failed to parse environment variables");
    let service = TextSvcImpl::new(&args)
        .await
        .expect("Failed to initialize TextService");

    println!("connected to text service");

    TextServiceServer::new(service)
}
