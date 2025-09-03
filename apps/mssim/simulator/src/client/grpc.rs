use anyhow::Result;
use tonic::{Request, Status};

use crate::proto::{
    ConfigurationRequest, simulator_orchestrator_client::SimulatorOrchestratorClient,
};

pub async fn submit_config_to_orchestrator(
    orchestrator_addr: &str,
    yaml_config: String,
) -> Result<String> {
    // Connect to the gRPC server
    let mut client = SimulatorOrchestratorClient::connect(format!("http://{}", orchestrator_addr))
        .await
        .map_err(|e| anyhow::anyhow!("Failed to connect to orchestrator: {}", e))?;

    // Prepare the request
    let request = Request::new(ConfigurationRequest {
        yaml_config,
        start_immediately: true,
    });

    // Send the request
    let response = client
        .submit_configuration(request)
        .await
        .map_err(|e: Status| anyhow::anyhow!("gRPC error: {}", e))?;

    let response = response.into_inner();

    // Return the simulation ID or error message
    if response.success {
        Ok(response.simulation_id)
    } else {
        anyhow::bail!("Failed to submit configuration: {}", response.message)
    }
}
