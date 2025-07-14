use anyhow::{Context, Result};
use reqwest::{Client, StatusCode};
use serde_json::Value;

#[allow(dead_code)]
pub async fn submit_config_via_http(server_addr: &str, json_str: String) -> Result<String> {
    // Create HTTP client
    let client = Client::new();

    // Prepare the request
    let response = client
        .post(&format!("http://{}/submit", server_addr))
        .header("Content-Type", "application/json")
        .body(json_str)
        .send()
        .await
        .context("Failed to send HTTP request")?;

    // Check response status
    if response.status() != StatusCode::OK {
        let error_msg = response
            .text()
            .await
            .context("Failed to read error response")?;
        anyhow::bail!("HTTP submission failed: {}", error_msg);
    }

    // Get response as text first
    let response_text = response
        .text()
        .await
        .context("Failed to read response body")?;

    // Then parse the text as JSON
    let response_body: Value =
        serde_json::from_str(&response_text).context("Failed to parse JSON response")?;

    // Extract simulation ID or error
    if let Some(success) = response_body.get("success").and_then(|v| v.as_bool()) {
        if success {
            if let Some(simulation_id) = response_body.get("simulation_id").and_then(|v| v.as_str())
            {
                Ok(simulation_id.to_string())
            } else {
                anyhow::bail!("Missing simulation_id in response")
            }
        } else {
            let error_msg = response_body
                .get("error")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            anyhow::bail!("Submission failed: {}", error_msg)
        }
    } else {
        anyhow::bail!("Invalid response format")
    }
}
