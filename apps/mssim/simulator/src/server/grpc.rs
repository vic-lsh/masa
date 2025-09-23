use anyhow::Result;
use std::sync::Arc;
use tonic::{transport::Server, Request, Response, Status};

use crate::proto::input_parser_server::{InputParser, InputParserServer};
use crate::proto::{ParseRequest, ParseResponse};

#[allow(dead_code)]
pub struct InputParserService {
    orchestrator_addr: Arc<String>,
}

#[tonic::async_trait]
impl InputParser for InputParserService {
    async fn parse_input(
        &self,
        _request: Request<ParseRequest>,
    ) -> Result<Response<ParseResponse>, Status> {
        return Err(Status::unimplemented(
            "Not yet ported to support new trace format from Alibaba",
        ));
        // let req = request.into_inner();

        // // Parse JSON
        // let config = json::parse_json_str(&req.json_config)
        //     .map_err(|e| Status::invalid_argument(format!("Invalid JSON: {}", e)))?;

        // // Validate config
        // validator::validate_config(&config)
        //     .map_err(|e| Status::invalid_argument(format!("Validation error: {}", e)))?;

        // // Generate YAML
        // let yaml_str = yaml::generate_simulator_yaml(&config)
        //     .map_err(|e| Status::internal(format!("YAML generation error: {}", e)))?;

        // // If forward flag is set, send to orchestrator
        // let simulation_id = if req.forward_to_orchestrator {
        //     match orchestrator_client::submit_config_to_orchestrator(
        //         &self.orchestrator_addr,
        //         yaml_str.clone(),
        //     )
        //     .await
        //     {
        //         Ok(id) => id,
        //         Err(e) => return Err(Status::internal(format!("Orchestrator error: {}", e))),
        //     }
        // } else {
        //     String::new()
        // };

        // Ok(Response::new(ParseResponse {
        //     success: true,
        //     yaml_config: yaml_str,
        //     simulation_id,
        //     error_message: String::new(),
        // }))
    }
}

pub async fn start_grpc_server(port: u16, orchestrator_addr: String) -> Result<()> {
    let addr = format!("0.0.0.0:{}", port).parse()?;
    let orchestrator_addr = Arc::new(orchestrator_addr);

    let service = InputParserService { orchestrator_addr };

    println!("Starting gRPC server on {}", addr);

    Server::builder()
        .add_service(InputParserServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}
