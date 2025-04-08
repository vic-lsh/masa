use tonic::{Request, Status};

use crate::url_shorten::{
    ComposeUrlsRequest, Url, GetExtendedUrlsRequest,
    url_shorten_service_client::UrlShortenServiceClient,
};

pub mod url_shorten {
    tonic::include_proto!("url_shorten");
}

pub struct UrlShortenClient {
    client: UrlShortenServiceClient<tonic::transport::Channel>,
}

impl UrlShortenClient {
    pub async fn connect(addr: String) -> Result<Self, Box<dyn std::error::Error>> {
        let client = UrlShortenServiceClient::connect(addr).await?;
        Ok(Self { client })
    }
    
    pub async fn compose_urls(
        &mut self,
        req_id: i64,
        urls: Vec<String>
    ) -> Result<Vec<Url>, Status> {
        let request = Request::new(ComposeUrlsRequest {
            req_id,
            urls,
        });
        
        let response = self.client.compose_urls(request).await?;
        let inner = response.into_inner();
        
        if let Some(exception) = inner.exception {
            return Err(Status::internal(exception.message));
        }
        
        Ok(inner.urls)
    }
    
    pub async fn get_extended_urls(
        &mut self,
        req_id: i64,
        shortened_urls: Vec<String>
    ) -> Result<Vec<String>, Status> {
        let request = Request::new(GetExtendedUrlsRequest {
            req_id,
            shortened_urls,
        });
        
        let response = self.client.get_extended_urls(request).await?;
        let inner = response.into_inner();
        
        if let Some(exception) = inner.exception {
            return Err(Status::internal(exception.message));
        }
        
        Ok(inner.expanded_urls)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = UrlShortenClient::connect("http://[::1]:50053".to_string()).await?;
    
    // Example data for testing
    let req_id = 12345;
    let urls = vec![
        "https://www.rust-lang.org".to_string(),
        "https://github.com/tokio-rs/tonic".to_string()
    ];
    
    // Test the compose_urls endpoint
    match client.compose_urls(req_id, urls).await {
        Ok(url_list) => {
            println!("Successfully composed {} shortened URLs:", url_list.len());
            for (i, url) in url_list.iter().enumerate() {
                println!("  {}: Shortened={}, Original={}", 
                    i+1, url.shortened_url, url.expanded_url);
            }
            
            // Getting extended URLs
            let shortened_urls: Vec<String> = url_list.iter()
                .map(|url| url.shortened_url.clone())
                .collect();
                
            match client.get_extended_urls(req_id, shortened_urls).await {
                Ok(expanded_urls) => {
                    println!("\nSuccessfully retrieved {} expanded URLs:", expanded_urls.len());
                    for (i, url) in expanded_urls.iter().enumerate() {
                        println!("  {}: {}", i+1, url);
                    }
                },
                Err(status) => {
                    eprintln!("Error getting extended URLs: {}", status);
                }
            }
        },
        Err(status) => {
            eprintln!("Error composing shortened URLs: {}", status);
        }
    }
    
    Ok(())
}