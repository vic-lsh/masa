use tonic::{Request, Response, Status};

use crate::media::{
    media_service_server::{MediaService, MediaServiceServer},
    ComposeMediaRequest, ComposeMediaResponse, ErrorCode, Media, ServiceException,
};

#[derive(Debug, Default)]
pub struct MediaServiceImpl {}

impl MediaServiceImpl {
    pub fn new() -> Self {
        Self {}
    }
}

#[tonic::async_trait]
impl MediaService for MediaServiceImpl {
    async fn compose_media(
        &self,
        request: Request<ComposeMediaRequest>,
    ) -> Result<Response<ComposeMediaResponse>, Status> {
        println!("Got a request: {:?}", request);

        let req = request.into_inner();

        // Validate input
        if req.media_types.len() != req.media_ids.len() {
            let exception = ServiceException {
                error_code: ErrorCode::SeThriftHandlerError as i32,
                message: "The lengths of media_id list and media_type list are not equal"
                    .to_string(),
            };

            return Ok(Response::new(ComposeMediaResponse {
                media: vec![],
                exception: Some(exception),
            }));
        }

        // Compose the media list
        let media = req
            .media_types
            .iter()
            .zip(req.media_ids.iter())
            .map(|(media_type, media_id)| Media {
                media_id: *media_id,
                media_type: media_type.clone(),
            })
            .collect();

        Ok(Response::new(ComposeMediaResponse {
            media,
            exception: None,
        }))
    }
}

pub fn create_service() -> MediaServiceServer<MediaServiceImpl> {
    let service = MediaServiceImpl::new();
    MediaServiceServer::new(service)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::media_service_client::MediaServiceClient;
    use tokio::sync::oneshot;
    use tonic::transport::Server;

    async fn start_test_server() -> (String, oneshot::Sender<()>) {
        let (tx, rx) = oneshot::channel();

        let listener =
            std::net::TcpListener::bind("[::1]:0").expect("Failed to bind to random port");
        let addr = listener.local_addr().expect("Failed to get local address");
        drop(listener);

        let service = create_service();

        let server = Server::builder()
            .add_service(service)
            .serve_with_shutdown(addr, async {
                rx.await.ok();
            });

        let server_addr = format!("http://{}", addr);

        tokio::spawn(async move {
            server.await.expect("Server failed");
        });

        // Give the server some time to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        (server_addr, tx)
    }

    #[tokio::test]
    async fn test_compose_media_success() {
        let (server_addr, shutdown_tx) = start_test_server().await;

        let mut client = MediaServiceClient::connect(server_addr)
            .await
            .expect("Failed to connect");

        // Test data
        let req_id = 12345;
        let media_types = vec!["photo".to_string(), "video".to_string()];
        let media_ids = vec![1001, 2002];

        // Make the request
        let request = tonic::Request::new(ComposeMediaRequest {
            req_id,
            media_types: media_types.clone(),
            media_ids: media_ids.clone(),
        });

        let response = client.compose_media(request).await.expect("Request failed");
        let response_inner = response.into_inner();

        assert!(
            response_inner.exception.is_none(),
            "Received unexpected exception: {:?}",
            response_inner.exception
        );

        // Assert the correct media was returned
        assert_eq!(response_inner.media.len(), 2);
        assert_eq!(response_inner.media[0].media_id, media_ids[0]);
        assert_eq!(response_inner.media[0].media_type, media_types[0]);
        assert_eq!(response_inner.media[1].media_id, media_ids[1]);
        assert_eq!(response_inner.media[1].media_type, media_types[1]);

        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
    }

    #[tokio::test]
    async fn test_compose_media_validation_error() {
        let (server_addr, shutdown_tx) = start_test_server().await;

        let mut client = MediaServiceClient::connect(server_addr)
            .await
            .expect("Failed to connect");

        // Test data with mismatched lengths to trigger validation error
        let req_id = 12345;
        let media_types = vec!["photo".to_string(), "video".to_string()];
        let media_ids = vec![1001];

        let request = tonic::Request::new(ComposeMediaRequest {
            req_id,
            media_types,
            media_ids,
        });

        let response = client.compose_media(request).await.expect("Request failed");
        let response_inner = response.into_inner();

        // Assert exception is present
        assert!(
            response_inner.exception.is_some(),
            "Expected validation exception"
        );
        if let Some(exception) = response_inner.exception {
            assert!(
                exception.message.contains("lengths"),
                "Expected error about lengths mismatch, got: {}",
                exception.message
            );
        }

        assert_eq!(response_inner.media.len(), 0);

        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
    }

    #[tokio::test]
    async fn test_empty_lists_case() {
        let (server_addr, shutdown_tx) = start_test_server().await;

        let mut client = MediaServiceClient::connect(server_addr)
            .await
            .expect("Failed to connect");

        // Test with empty lists
        let req_id = 12345;
        let empty_media_types: Vec<String> = Vec::new();
        let empty_media_ids: Vec<i64> = Vec::new();

        let request = tonic::Request::new(ComposeMediaRequest {
            req_id,
            media_types: empty_media_types,
            media_ids: empty_media_ids,
        });

        let response = client.compose_media(request).await.expect("Request failed");
        let response_inner = response.into_inner();

        // Should succeed with empty result
        assert!(
            response_inner.exception.is_none(),
            "Expected no exception with empty lists"
        );
        assert_eq!(response_inner.media.len(), 0, "Expected empty result");

        shutdown_tx
            .send(())
            .expect("Failed to send shutdown signal");
    }
}
