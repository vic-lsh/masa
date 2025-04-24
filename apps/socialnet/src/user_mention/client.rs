use tonic::{Request, Status};

use user_mention_service::{user_mention_service_client::UserMentionServiceClient, ComposeUserMentionRequest, UserMention};

pub mod user_mention_service {
    tonic::include_proto!("usermention");
}

pub struct UserMentionClient {
    client: UserMentionServiceClient<tonic::transport::Channel>,
}

impl UserMentionClient {
    pub async fn connect(addr: String) -> Result<Self, Box<dyn std::error::Error>> {
        let client = UserMentionServiceClient::connect(addr).await?;
        Ok(Self { client })
    }

    pub async fn compose_user_mentions(
        &mut self,
        req_id: u64,
        usernames: Vec<String>,
    ) -> Result<Vec<UserMention>, Status> {
        let request = Request::new(ComposeUserMentionRequest {
            req_id,
            usernames,
        });

        let response = self.client.compose_user_mentions(request).await?;
        let inner = response.into_inner();

        if let Some(exception) = inner.exception {
            return Err(Status::internal(exception.message));
        }

        Ok(inner.user_mentions)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = UserMentionClient::connect("http://[::1]:50052".to_string()).await?;

    let req_id = 12345;
    let user_names = vec!["adam".to_string(), "alice".to_string(), "jasper".to_string()];

    match client.compose_user_mentions(req_id, user_names).await {
        Ok(user_mentions) => {
            println!("Successfully fetch {} user mentions :", user_mentions.len());
            for (i, user_mention) in user_mentions.iter().enumerate() {
                println!(
                    "  {}: User_ID={}, User_Name={}",
                    i + 1,
                    user_mention.user_id,
                    user_mention.username
                );
            }
        }
        Err(status) => {
            eprintln!("Error calling user_mention service: {}", status);
        }
    }

    let req_id = 12345;
    let user_names = vec!["adam".to_string(), "alice".to_string(), "jasper".to_string()];

    match client.compose_user_mentions(req_id, user_names).await {
        Ok(user_mentions) => {
            println!("Successfully fetch {} user mentions :", user_mentions.len());
            for (i, user_mention) in user_mentions.iter().enumerate() {
                println!(
                    "  {}: User_ID={}, User_Name={}",
                    i + 1,
                    user_mention.user_id,
                    user_mention.username
                );
            }
        }
        Err(status) => {
            eprintln!("Error calling user_mention service: {}", status);
        }
    }

    Ok(())
}
