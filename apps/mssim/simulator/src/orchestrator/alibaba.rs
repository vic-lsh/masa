use super::{Backend, docker, kubernetes};
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use sim_config::deployment::{Deployment, ServiceDiscoveryInfo};
use sim_config::svc::ServiceName;
use sim_config::trace::TraceConfig;
use sim_config::SimulatorConfig;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};
use tracing::{debug, error, info};

const CONTAINER_CPU_LIMIT: usize = 1;
const SERVICE_CONFIG_DIR: &str = "./service_configs";

#[allow(dead_code)]
#[derive(Deserialize, Debug, serde::Serialize, Clone)]
pub struct ErrorRate {
    #[serde(rename = "distribution_type")]
    pub rate_type: String,
    pub parameters: HashMap<String, f64>,
}

pub fn assign_ports(
    service_names: impl Iterator<Item = ServiceName>,
) -> Result<HashMap<ServiceName, u16>> {
    info!("Assigning ports to services.");
    let mut port_assignments = HashMap::new();
    let mut available_ports = (50051..60000).collect::<Vec<u16>>();

    for service_name in service_names {
        if let Some(index) = available_ports.pop() {
            port_assignments.insert(service_name.clone(), index);
            debug!("Assigned port {} to service {}", index, service_name);
        } else {
            error!("Ran out of available ports.");
            return Err(anyhow!("Ran out of available ports."));
        }
    }

    info!("Port assignment complete: {:?}", port_assignments);
    Ok(port_assignments)
}

pub fn generate_service_configs(
    port_assignments: &HashMap<ServiceName, u16>,
    sim_cfg: &SimulatorConfig,
) -> Result<()> {
    info!("Generating service-specific configuration files.");
    let config_dir = PathBuf::from(SERVICE_CONFIG_DIR);

    fs::create_dir_all(&config_dir)
        .with_context(|| format!("Failed to create directory: {:?}", config_dir))?;

    let service_config_path = config_dir.join("deployment.json");

    let deployment = make_deployment_config(port_assignments, sim_cfg);

    deployment
        .export_to_file(&service_config_path)
        .map_err(|_| {
            anyhow!(
                "Failed to write deployment config to {:?}",
                service_config_path
            )
        })?;

    info!(
        "Created config file containing all service configurations at {:?}",
        service_config_path
    );

    Ok(())
}

fn make_deployment_config(
    port_assignments: &HashMap<ServiceName, u16>,
    sim_cfg: &SimulatorConfig,
) -> Deployment {
    let mut deployment = Deployment::new();

    for (service_name, port) in port_assignments {
        deployment.add_service(
            service_name.clone(),
            ServiceDiscoveryInfo {
                ip: service_name.to_string(),
                port: *port,
                replicas: sim_cfg.replicas.get(service_name).unwrap_or(&1) as usize,
            },
        );
    }

    deployment
}

pub async fn launch_simulation_from_yaml(
    config: TraceConfig,
    trace_dir: &PathBuf,
    sim_config: SimulatorConfig,
    replay_path: Option<&Path>,
    backend: Backend,
) -> Result<()> {
    let port_assignments = assign_ports(config.call_graph.services().into_iter())?;
    info!("Port assignments: {:?}", port_assignments);

    generate_service_configs(&port_assignments, &sim_config)?;

    let service_config_dir = Path::new(SERVICE_CONFIG_DIR);
    let workspace_root = workspace_root();

    match backend {
        Backend::DockerCompose => {
            docker::generate_compose(
                &config,
                &port_assignments,
                trace_dir.as_path(),
                service_config_dir,
                workspace_root.as_path(),
                CONTAINER_CPU_LIMIT,
                &sim_config,
                replay_path,
            )?;

            docker::run()?;

            tokio::signal::ctrl_c().await?;
            info!("Received termination signal.");

            docker::stop()?;

            info!("Collecting and reporting output...");
            Ok(())
        }
        Backend::Kubernetes => {
            let images = kubernetes::build_images(workspace_root.as_path())?;
            let manifest_path = kubernetes::generate_manifest(
                &config,
                &port_assignments,
                trace_dir.as_path(),
                service_config_dir,
                &images,
                CONTAINER_CPU_LIMIT,
            )?;

            kubernetes::apply(&manifest_path)?;

            tokio::signal::ctrl_c().await?;
            info!("Received termination signal.");

            kubernetes::delete(&manifest_path)?;

            info!("Collecting and reporting output...");
            Ok(())
        }
    }
}

fn workspace_root() -> PathBuf {
    env!("CARGO_WORKSPACE_DIR").into()
}
