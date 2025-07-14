use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;
use warp::{Filter, Rejection, Reply};

use crate::client::grpc;
use crate::generator::yaml;
use crate::parser::json;
use crate::validator;

pub async fn start_http_server(port: u16, orchestrator_addr: String) -> Result<()> {
    let orchestrator = Arc::new(orchestrator_addr);

    // POST /submit endpoint for JSON submission
    let submit = warp::path("submit")
        .and(warp::post())
        .and(warp::body::content_length_limit(1024 * 1024)) // 1MB limit
        .and(warp::body::json())
        .and(with_orchestrator(orchestrator))
        .and_then(handle_submit);

    // Healthcheck endpoint
    let health = warp::path("health")
        .and(warp::get())
        .map(|| warp::reply::json(&serde_json::json!({"status": "ok"})));

    let routes = submit.or(health);

    println!("Starting HTTP server on port {}", port);
    warp::serve(routes).run(([0, 0, 0, 0], port)).await;

    Ok(())
}

fn with_orchestrator(
    orchestrator: Arc<String>,
) -> impl Filter<Extract = (Arc<String>,), Error = std::convert::Infallible> + Clone {
    warp::any().map(move || orchestrator.clone())
}

async fn handle_submit(
    json_input: Value,
    orchestrator: Arc<String>,
) -> Result<impl Reply, Rejection> {
    // Convert JSON value to string
    let json_str = json_input.to_string();

    // Parse JSON
    match json::parse_json_str(&json_str) {
        Ok(config) => {
            // Validate config
            if let Err(err) = validator::validate_config(&config) {
                return Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({
                        "success": false,
                        "error": format!("Validation error: {}", err)
                    })),
                    warp::http::StatusCode::BAD_REQUEST,
                ));
            }

            // Generate YAML
            match yaml::generate_simulator_yaml(&config) {
                Ok(yaml_str) => {
                    // Submit to orchestrator
                    match grpc::submit_config_to_orchestrator(&orchestrator, yaml_str).await {
                        Ok(simulation_id) => Ok(warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "success": true,
                                "simulation_id": simulation_id
                            })),
                            warp::http::StatusCode::OK,
                        )),
                        Err(err) => Ok(warp::reply::with_status(
                            warp::reply::json(&serde_json::json!({
                                "success": false,
                                "error": format!("Orchestrator error: {}", err)
                            })),
                            warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                        )),
                    }
                }
                Err(err) => Ok(warp::reply::with_status(
                    warp::reply::json(&serde_json::json!({
                        "success": false,
                        "error": format!("YAML generation error: {}", err)
                    })),
                    warp::http::StatusCode::INTERNAL_SERVER_ERROR,
                )),
            }
        }
        Err(err) => Ok(warp::reply::with_status(
            warp::reply::json(&serde_json::json!({
                "success": false,
                "error": format!("JSON parsing error: {}", err)
            })),
            warp::http::StatusCode::BAD_REQUEST,
        )),
    }
}
