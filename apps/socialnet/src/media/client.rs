use tonic::{Request, Status};

use crate::media::{
    ComposeMediaRequest, Media,
    media_service_client::MediaServiceClient,
};

pub mod media {
    tonic::include_proto!("media");
}

pub struct MediaClient {
    client: MediaServiceClient<tonic::transport::Channel>,
}

impl MediaClient {
    pub async fn connect(addr: String) -> Result<Self, Box<dyn std::error::Error>> {
        let client = MediaServiceClient::connect(addr).await?;
        Ok(Self { client })
    }
    
    pub async fn compose_media(
        &mut self,
        req_id: i64,
        media_types: Vec<String>,
        media_ids: Vec<i64>
    ) -> Result<Vec<Media>, Status> {
        let request = Request::new(ComposeMediaRequest {
            req_id,
            media_types,
            media_ids
        });
        
        let response = self.client.compose_media(request).await?;
        let inner = response.into_inner();
        
        if let Some(exception) = inner.exception {
            return Err(Status::internal(exception.message));
        }
        
        Ok(inner.media)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = MediaClient::connect("http://[::1]:50052".to_string()).await?;
    
    let req_id = 12345;
    let media_types = vec!["photo".to_string(), "video".to_string()];
    let media_ids = vec![1001, 2002];
    
    match client.compose_media(req_id, media_types, media_ids).await {
        Ok(media_list) => {
            println!("Successfully composed {} media items:", media_list.len());
            for (i, media) in media_list.iter().enumerate() {
                println!("  {}: ID={}, Type={}", i+1, media.media_id, media.media_type);
            }
        },
        Err(status) => {
            eprintln!("Error calling media service: {}", status);
        }
    }
    
    Ok(())
}