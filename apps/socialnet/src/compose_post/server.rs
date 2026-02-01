use std::collections::HashMap;
use std::convert::TryFrom;
use std::net::SocketAddr;

use chrono::Utc;
use log::info;
use once_cell::sync::Lazy;
use regex::Regex;
use tonic::async_trait;
use tonic::transport::masa_channel::LoadBalancedChannel;
use tonic::transport::Server;
use tonic::{Request, Response, Status};
use structopt::StructOpt;

use crate::compose_post::compose_post_service_server::{
    ComposePostService, ComposePostServiceServer,
};
use crate::compose_post::{ComposePostRequest, ComposePostResponse};
use crate::home_timeline::home_timeline_service_client::HomeTimelineServiceClient;
use crate::home_timeline::WriteHomeTimelineRequest;
use crate::media::media_service_client::MediaServiceClient;
use crate::media::{ComposeMediaRequest, Media};
use crate::post_storage::post_storage_service_client::PostStorageServiceClient;
use crate::post_storage::StorePostRequest;
use crate::text_service::text_service_client::TextServiceClient;
use crate::text_service::{TextReply, TextRequest};
use crate::url_shorten::url_shorten_service_client::UrlShortenServiceClient;
use crate::url_shorten::{ComposeUrlsRequest, Url};
use crate::user::user_service_client::UserServiceClient;
use crate::user::{ComposeCreatorWithUserIdRequest, Creator};
use crate::user_timeline::user_timeline_service_client::UserTimelineServiceClient;
use crate::user_timeline::{Post, WriteUserTimelineRequest};
use crate::usermention::user_mention_service_client::UserMentionServiceClient;
use crate::usermention::{ComposeUserMentionRequest, UserMention};

mod uniqueid {
    tonic::include_proto!("uniqueidservice");
}

use uniqueid::unique_id_service_client::UniqueIdServiceClient;
use uniqueid::UniqueIdRequest;

/// Configuration required to bootstrap the ComposePost gRPC server.
#[derive(Clone, Debug, StructOpt)]
pub struct Args {
    /// gRPC listen address for the ComposePost service, e.g. `0.0.0.0:50064`.
    #[structopt(long, env = "COMPOSE_POST_LISTEN_ADDR", default_value = "0.0.0.0:8080")]
    pub listen_addr: String,

    /// Endpoint of the PostStorage gRPC service.
    #[structopt(long, env = "POST_STORAGE_IP", default_value = "post-storage-service")]
    pub post_storage_ip: String,
    #[structopt(long, env = "POST_STORAGE_PORT", default_value = "8080")]
    pub post_storage_port: u16,
    #[structopt(long, env = "POST_STORAGE_REPLICAS", default_value = "1")]
    pub post_storage_replicas: u8,

    /// Endpoint of the UserTimeline gRPC service.
    #[structopt(long, env = "USER_TIMELINE_IP", default_value = "user-timeline-service")]
    pub user_timeline_ip: String,
    #[structopt(long, env = "USER_TIMELINE_PORT", default_value = "8080")]
    pub user_timeline_port: u16,
    #[structopt(long, env = "USER_TIMELINE_REPLICAS", default_value = "1")]
    pub user_timeline_replicas: u8,

    /// Endpoint of the HomeTimeline gRPC service.
    #[structopt(long, env = "HOME_TIMELINE_IP", default_value = "home-timeline-service")]
    pub home_timeline_ip: String,
    #[structopt(long, env = "HOME_TIMELINE_PORT", default_value = "8080")]
    pub home_timeline_port: u16,
    #[structopt(long, env = "HOME_TIMELINE_REPLICAS", default_value = "1")]
    pub home_timeline_replicas: u8,

    /// Endpoint of the User gRPC service.
    #[structopt(long, env = "USER_SERVICE_IP", default_value = "user-service")]
    pub user_service_ip: String,
    #[structopt(long, env = "USER_SERVICE_PORT", default_value = "8080")]
    pub user_service_port: u16,
    #[structopt(long, env = "USER_SERVICE_REPLICAS", default_value = "1")]
    pub user_service_replicas: u8,

    /// Endpoint of the UniqueId gRPC service.
    #[structopt(long, env = "UNIQUE_ID_SERVICE_IP", default_value = "socialnet-unique-id-service")]
    pub unique_id_ip: String,
    #[structopt(long, env = "UNIQUE_ID_SERVICE_PORT", default_value = "8080")]
    pub unique_id_port: u16,
    #[structopt(long, env = "UNIQUE_ID_SERVICE_REPLICAS", default_value = "1")]
    pub unique_id_replicas: u8,

    /// Endpoint of the Media gRPC service.
    #[structopt(long, env = "MEDIA_SERVICE_IP", default_value = "media-service")]
    pub media_service_ip: String,
    #[structopt(long, env = "MEDIA_SERVICE_PORT", default_value = "8080")]
    pub media_service_port: u16,
    #[structopt(long, env = "MEDIA_SERVICE_REPLICAS", default_value = "1")]
    pub media_service_replicas: u8,

    /// Endpoint of the Text gRPC service.
    #[structopt(long, env = "TEXT_SERVICE_IP", default_value = "text-service")]
    pub text_service_ip: String,
    #[structopt(long, env = "TEXT_SERVICE_PORT", default_value = "8080")]
    pub text_service_port: u16,
    #[structopt(long, env = "TEXT_SERVICE_REPLICAS", default_value = "1")]
    pub text_service_replicas: u8,

    /// Endpoint of the UserMention gRPC service.
    #[structopt(long, env = "USER_MENTION_SERVICE_IP", default_value = "user-mention-service")]
    pub user_mention_service_ip: String,
    #[structopt(long, env = "USER_MENTION_SERVICE_PORT", default_value = "8080")]
    pub user_mention_service_port: u16,
    #[structopt(long, env = "USER_MENTION_SERVICE_REPLICAS", default_value = "1")]
    pub user_mention_service_replicas: u8,

    /// Endpoint of the UrlShorten gRPC service.
    #[structopt(long, env = "URL_SHORTEN_SERVICE_IP", default_value = "url-shorten-service")]
    pub url_shorten_service_ip: String,
    #[structopt(long, env = "URL_SHORTEN_SERVICE_PORT", default_value = "8080")]
    pub url_shorten_service_port: u16,
    #[structopt(long, env = "URL_SHORTEN_SERVICE_REPLICAS", default_value = "1")]
    pub url_shorten_service_replicas: u8,
}

#[derive(Clone)]
pub struct ComposePostServiceImpl {
    post_storage: LoadBalancedChannel,
    user_timeline: LoadBalancedChannel,
    home_timeline: LoadBalancedChannel,
    user_service: LoadBalancedChannel,
    unique_id: LoadBalancedChannel,
    media_service: LoadBalancedChannel,
    text_service: LoadBalancedChannel,
    user_mention: LoadBalancedChannel,
    url_shorten: LoadBalancedChannel,
}

impl ComposePostServiceImpl {
    pub async fn new(args: &Args) -> Result<Self, Box<dyn std::error::Error>> {
        let post_storage = create_channel_for_service(
            args.post_storage_ip.clone(),
            args.post_storage_port,
            args.post_storage_replicas,
        )
        .await?;

        let user_timeline = create_channel_for_service(
            args.user_timeline_ip.clone(),
            args.user_timeline_port,
            args.user_timeline_replicas,
        )
        .await?;

        let home_timeline = create_channel_for_service(
            args.home_timeline_ip.clone(),
            args.home_timeline_port,
            args.home_timeline_replicas,
        )
        .await?;

        let user_service = create_channel_for_service(
            args.user_service_ip.clone(),
            args.user_service_port,
            args.user_service_replicas,
        )
        .await?;

        let text_service = create_channel_for_service(
            args.text_service_ip.clone(),
            args.text_service_port,
            args.text_service_replicas,
        )
        .await?;

        let user_mention = create_channel_for_service(
            args.user_mention_service_ip.clone(),
            args.user_mention_service_port,
            args.user_mention_service_replicas,
        )
        .await?;

        let url_shorten = create_channel_for_service(
            args.url_shorten_service_ip.clone(),
            args.url_shorten_service_port,
            args.url_shorten_service_replicas,
        )
        .await?;

        let unique_id = create_channel_for_service(
            args.unique_id_ip.clone(),
            args.unique_id_port,
            args.unique_id_replicas,
        )
        .await?;

        let media_service = create_channel_for_service(
            args.media_service_ip.clone(),
            args.media_service_port,
            args.media_service_replicas,
        )
        .await?;

        Ok(Self {
            post_storage,
            user_timeline,
            home_timeline,
            user_service,
            unique_id,
            media_service,
            text_service,
            user_mention,
            url_shorten,
        })
    }

    async fn compose_creator(
        &self,
        req_id: i64,
        user_id: i64,
        username: &str,
        carrier: &HashMap<String, String>,
    ) -> Result<Creator, Status> {
        let mut client = UserServiceClient::new(self.user_service.clone());
        let request = ComposeCreatorWithUserIdRequest {
            req_id,
            user_id,
            username: username.to_string(),
            carrier: carrier.clone(),
        };
        let response = client
            .compose_creator_with_user_id(Request::new(request))
            .await?;
        let inner = response.into_inner();
        inner
            .creator
            .ok_or_else(|| Status::internal("User service response missing creator"))
    }

    async fn compose_text(&self, text: String) -> Result<TextReply, Status> {
        let mut client = TextServiceClient::new(self.text_service.clone());
        let request = TextRequest { text };
        let response = client.compose_text(Request::new(request)).await?;
        Ok(response.into_inner())
    }

    async fn compose_media(
        &self,
        req_id: i64,
        media_types: Vec<String>,
        media_ids: Vec<i64>,
    ) -> Result<Vec<Media>, Status> {
        if media_types.len() != media_ids.len() {
            return Err(Status::invalid_argument(
                "media_types and media_ids must have the same length",
            ));
        }
        if media_types.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = MediaServiceClient::new(self.media_service.clone());
        let request = ComposeMediaRequest {
            req_id,
            media_types,
            media_ids,
        };
        let mut response = client
            .compose_media(Request::new(request))
            .await?
            .into_inner();
        if let Some(exception) = response.exception.take() {
            return Err(Status::internal(exception.message));
        }
        Ok(response.media)
    }

    async fn compose_unique_id(&self, req_id: i64) -> Result<i64, Status> {
        if req_id < 0 {
            return Err(Status::invalid_argument("req_id must be non-negative"));
        }
        let mut client = UniqueIdServiceClient::new(self.unique_id.clone());
        let request = UniqueIdRequest { id: req_id as u64 };
        let response = client.compose_unique_id(Request::new(request)).await?;
        Ok(response.into_inner().message as i64)
    }

    async fn compose_user_mentions(
        &self,
        req_id: u64,
        usernames: Vec<String>,
    ) -> Result<Vec<UserMention>, Status> {
        if usernames.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = UserMentionServiceClient::new(self.user_mention.clone());
        let request = ComposeUserMentionRequest { req_id, usernames };
        let mut response = client
            .compose_user_mentions(Request::new(request))
            .await?
            .into_inner();
        if let Some(exception) = response.exception.take() {
            return Err(Status::internal(exception.message));
        }
        Ok(response.user_mentions)
    }

    async fn compose_urls(&self, req_id: i64, urls: Vec<String>) -> Result<Vec<Url>, Status> {
        if urls.is_empty() {
            return Ok(Vec::new());
        }
        let mut client = UrlShortenServiceClient::new(self.url_shorten.clone());
        let request = ComposeUrlsRequest { req_id, urls };
        let mut response = client
            .compose_urls(Request::new(request))
            .await?
            .into_inner();
        if let Some(exception) = response.exception.take() {
            return Err(Status::internal(exception.message));
        }
        Ok(response.urls)
    }

    async fn upload_post(
        &self,
        req_id: i64,
        post: Post,
        carrier: HashMap<String, String>,
    ) -> Result<(), Status> {
        let mut client = PostStorageServiceClient::new(self.post_storage.clone());
        let request = StorePostRequest {
            req_id,
            post: Some(post),
            carrier,
        };
        client.store_post(Request::new(request)).await?;
        Ok(())
    }

    async fn upload_user_timeline(
        &self,
        req_id: i64,
        post_id: i64,
        user_id: i64,
        timestamp: i64,
        carrier: HashMap<String, String>,
    ) -> Result<(), Status> {
        let mut client = UserTimelineServiceClient::new(self.user_timeline.clone());
        let request = WriteUserTimelineRequest {
            req_id,
            post_id,
            user_id,
            timestamp,
            carrier,
        };
        client.write_user_timeline(Request::new(request)).await?;
        Ok(())
    }

    async fn upload_home_timeline(
        &self,
        req_id: i64,
        post_id: i64,
        user_id: i64,
        timestamp: i64,
        user_mentions_id: Vec<i64>,
        carrier: HashMap<String, String>,
    ) -> Result<(), Status> {
        let mut client = HomeTimelineServiceClient::new(self.home_timeline.clone());
        let request = WriteHomeTimelineRequest {
            req_id,
            post_id,
            user_id,
            timestamp,
            user_mentions_id,
            carrier,
        };
        client.write_home_timeline(Request::new(request)).await?;
        Ok(())
    }
}

#[async_trait]
impl ComposePostService for ComposePostServiceImpl {
    async fn compose_post(
        &self,
        request: Request<ComposePostRequest>,
    ) -> Result<Response<ComposePostResponse>, Status> {
        let inner = request.into_inner();
        let ComposePostRequest {
            req_id,
            username,
            user_id,
            text,
            media_ids,
            media_types,
            post_type,
            carrier,
        } = inner;

        let req_id_u64 = u64::try_from(req_id)
            .map_err(|_| Status::invalid_argument("req_id must be non-negative"))?;

        let mention_usernames = extract_usernames(&text);
        let url_candidates = extract_urls(&text);

        let text_future = self.compose_text(text.clone());
        let creator_future = self.compose_creator(req_id, user_id, &username, &carrier);
        let media_future = self.compose_media(req_id, media_types.clone(), media_ids.clone());
        let unique_id_future = self.compose_unique_id(req_id);
        let mention_future = self.compose_user_mentions(req_id_u64, mention_usernames.clone());
        let urls_future = self.compose_urls(req_id, url_candidates.clone());

        let (text_reply, creator, media, post_id, mentions, urls) = tokio::try_join!(
            text_future,
            creator_future,
            media_future,
            unique_id_future,
            mention_future,
            urls_future,
        )?;

        let timestamp = Utc::now().timestamp_millis();

        let final_text = if text_reply.updated_text.is_empty() {
            text
        } else {
            text_reply.updated_text
        };

        let mut mention_ids = Vec::with_capacity(mentions.len());
        for mention in &mentions {
            let id = i64::try_from(mention.user_id)
                .map_err(|_| Status::internal("user_id overflowed i64 range"))?;
            mention_ids.push(id);
        }

        let post = Post {
            post_id,
            creator: Some(creator),
            req_id,
            text: final_text,
            user_mentions: mentions.clone(),
            media,
            urls,
            timestamp,
            post_type,
        };

        // Ensure the post is persisted before publishing timelines.
        self.upload_post(req_id, post.clone(), carrier.clone())
            .await?;
        tokio::try_join!(
            self.upload_user_timeline(req_id, post_id, user_id, timestamp, carrier.clone(),),
            self.upload_home_timeline(req_id, post_id, user_id, timestamp, mention_ids, carrier,),
        )?;

        info!(
            "compose_post completed for req_id={} post_id={} user_id={}",
            req_id, post_id, user_id
        );

        Ok(Response::new(ComposePostResponse {}))
    }
}

pub async fn run(args: Args) -> Result<(), Box<dyn std::error::Error>> {
    println!("creating service....");
    let addr: SocketAddr = args.listen_addr.parse()?;
    let service_impl = ComposePostServiceImpl::new(&args).await?;
    println!("created service");
    // info!("ComposePost service listening on {}", addr);
    println!("ComposePost service listening on {}", addr);

    Server::builder()
        .add_service(ComposePostServiceServer::new(service_impl))
        .serve(addr)
        .await?;
    Ok(())
}

async fn create_channel_for_service(
    ip: String,
    port: u16,
    replicas: u8, // We rename 'replicas' to '_replicas' to ignore the value
) -> Result<LoadBalancedChannel, Box<dyn std::error::Error>> {
    Ok(LoadBalancedChannel::new(ip, port, replicas).await)
}

fn extract_usernames(text: &str) -> Vec<String> {
    static MENTION_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"@[A-Za-z0-9-_]+").unwrap());
    MENTION_RE
        .find_iter(text)
        .map(|mat| mat.as_str()[1..].to_string())
        .collect()
}

fn extract_urls(text: &str) -> Vec<String> {
    static URL_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(http://|https://)([A-Za-z0-9_!~*'().&=+$%-]+)").unwrap());
    URL_RE
        .find_iter(text)
        .map(|mat| mat.as_str().to_string())
        .collect()
}