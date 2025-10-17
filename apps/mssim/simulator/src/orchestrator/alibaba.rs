use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use sim_config::deployment::{Deployment, ServiceDiscoveryInfo};
use sim_config::svc::ServiceName;
use sim_config::trace::TraceConfig;
use sim_config::{PROJECT_NAME, SimulatorConfig};
use std::{
    collections::HashMap,
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};
use tracing::{debug, error, info};
use yaml_rust::yaml::Hash;
use yaml_rust::{Yaml, YamlEmitter};

const LOADGEN_SERVICE_NAME: &str = "load_generator";

/// Hard-coded name of the service that is the root of the call graph
///
// TODO: make this configurable
const FRONTEND_SERVICE_NAME: &str = "USER";
const LOADGEN_OUTPUT_MOUNT: &str = "/app/loadgen_output";
const CONTAINER_CPU_LIMIT: usize = 1;
const CONTAINER_MEM_LIMIT: &str = "512MB";

const DEFAULT_SVC_PORT: u16 = 50051;

#[allow(dead_code)]
#[derive(Deserialize, Debug, serde::Serialize, Clone)] // Added serde::Serialize and Clone
pub struct ErrorRate {
    #[serde(rename = "distribution_type")]
    // This maps the YAML key 'type' to the Rust field 'distribution_type'
    pub rate_type: String,
    pub parameters: HashMap<String, f64>, // This maps the YAML key 'parameters' to a HashMap
}

// New function to generate individual config files for each service
pub fn generate_service_configs(
    services: impl Iterator<Item = ServiceName>,
    sim_cfg: &SimulatorConfig,
) -> Result<Deployment> {
    info!("Generating service-specific configuration files.");
    let config_dir = PathBuf::from("./service_configs"); // Directory to store individual configs

    // Create the config directory if it doesn't exist
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("Failed to create directory: {:?}", config_dir))?;

    // Define the path for the single config file
    let mut service_config_path = config_dir.clone();
    let output_filename = "deployment.json";
    service_config_path.push(output_filename);

    let deployment = make_deployment_config(services, sim_cfg);

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

    Ok(deployment)
}

fn make_deployment_config(
    services: impl Iterator<Item = ServiceName>,
    sim_cfg: &SimulatorConfig,
) -> Deployment {
    let mut deployment = Deployment::new();

    for service_name in services {
        println!("Service: {}, Port: {}", &service_name, DEFAULT_SVC_PORT);

        deployment.add_service(
            service_name.clone(),
            ServiceDiscoveryInfo {
                ip: format!("{}-{}", PROJECT_NAME, &service_name),
                port: DEFAULT_SVC_PORT,
                replicas: sim_cfg.replicas.get(&service_name).unwrap_or(1) as usize,
            },
        );
    }

    deployment
}

pub fn generate_docker_compose(
    config: &TraceConfig,
    trace_dir: &PathBuf,
    sim_cfg: &SimulatorConfig,
    deployment: &Deployment,
    replay_path: Option<&Path>,
) -> Result<()> {
    info!("Generating docker-compose.yml file.");

    let mut services_hash = Hash::new();
    for service_name in config.call_graph.services() {
        let svc_info = deployment
            .services
            .get(&service_name)
            .ok_or_else(|| anyhow::anyhow!("Service not found in deployment: {}", service_name))?;
        let svc_port = svc_info.port;

        let service_def = make_service_def(&service_name, svc_port, sim_cfg, trace_dir);
        services_hash.insert(Yaml::String(service_name.to_string()), service_def);
    }

    let loadgen_config = make_load_generator_config_yaml(trace_dir, deployment, replay_path)?;
    services_hash.insert(Yaml::String(LOADGEN_SERVICE_NAME.into()), loadgen_config);

    let doc = make_docker_compose_doc(services_hash);

    let mut output_string = String::new();
    let mut emitter = YamlEmitter::new(&mut output_string);
    emitter.dump(&doc).unwrap();

    let compose_path = PathBuf::from("./docker-compose.yml");

    fs::write(&compose_path, output_string).with_context(|| {
        format!(
            "Failed to write docker-compose.yml file to {:?}",
            compose_path
        )
    })?;

    info!("docker-compose.yml file generated successfully.");

    Ok(())
}

fn make_docker_compose_doc(services: Hash) -> Yaml {
    let mut doc_hash = Hash::new();
    doc_hash.insert(Yaml::String("services".into()), Yaml::Hash(services));
    doc_hash.insert(Yaml::String("networks".into()), make_networks_def());
    Yaml::Hash(doc_hash)
}

fn make_networks_def() -> Yaml {
    let mut networks_def = Hash::new();
    let mut microservice_net_def = Hash::new();
    microservice_net_def.insert(Yaml::String("driver".into()), Yaml::String("bridge".into()));
    networks_def.insert(
        Yaml::String("microservice_net".into()),
        Yaml::Hash(microservice_net_def),
    );
    Yaml::Hash(networks_def)
}

fn make_service_def(
    service_name: &ServiceName,
    svc_port: u16,
    sim_cfg: &SimulatorConfig,
    trace_dir: &PathBuf,
) -> Yaml {
    let mut service_def = Hash::new();

    service_def.insert(Yaml::String("build".into()), make_build_def(svc_port));
    let replica_count = sim_cfg.replicas.get(service_name).unwrap_or(1);
    service_def.insert(
        Yaml::String("scale".into()),
        Yaml::Integer(replica_count.into()),
    );
    service_def.insert(Yaml::String("deploy".into()), make_deploy_def());
    service_def.insert(
        Yaml::String("environment".into()),
        make_environment_def(service_name, svc_port),
    );
    service_def.insert(Yaml::String("volumes".into()), make_volumes_def(trace_dir));
    service_def.insert(
        Yaml::String("networks".into()),
        Yaml::Array(vec![Yaml::String("microservice_net".into())]),
    );

    Yaml::Hash(service_def)
}

fn make_build_def(svc_port: u16) -> Yaml {
    let mut build_def = Hash::new();
    build_def.insert(
        Yaml::String("context".into()),
        Yaml::String(workspace_root().to_string_lossy().to_string()),
    );
    let dockerfile_path = workspace_root().join("apps/mssim/generic-service/Dockerfile");
    build_def.insert(
        Yaml::String("dockerfile".into()),
        Yaml::String(dockerfile_path.to_string_lossy().to_string()),
    );
    let mut build_args = Hash::new();
    build_args.insert(
        Yaml::String("SERVICE_CONTAINER_PORT".into()),
        Yaml::String(svc_port.to_string()),
    );

    if let Ok(feature) = env::var("FEATURE") {
        build_args.insert(Yaml::String("FEATURE".into()), Yaml::String(feature));
    } else {
        build_args.insert(Yaml::String("FEATURE".into()), Yaml::String("fifo".into()));
    }

    build_def.insert(Yaml::String("args".into()), Yaml::Hash(build_args));
    Yaml::Hash(build_def)
}

fn make_deploy_def() -> Yaml {
    let mut deploy_def = Hash::new();
    let mut resources_def = Hash::new();
    let mut limits_def = Hash::new();
    limits_def.insert(
        Yaml::String("cpus".into()),
        Yaml::String(CONTAINER_CPU_LIMIT.to_string()),
    );
    limits_def.insert(
        Yaml::String("memory".into()),
        Yaml::String(CONTAINER_MEM_LIMIT.to_string()),
    );
    resources_def.insert(Yaml::String("limits".into()), Yaml::Hash(limits_def));
    deploy_def.insert(Yaml::String("resources".into()), Yaml::Hash(resources_def));
    Yaml::Hash(deploy_def)
}

fn make_environment_def(service_name: &ServiceName, svc_port: u16) -> Yaml {
    let mut environment = Hash::new();
    environment.insert(
        Yaml::String("SERVICE_NAME".into()),
        Yaml::String(service_name.into()),
    );
    environment.insert(
        Yaml::String("SERVICE_PORT".into()),
        Yaml::String(svc_port.to_string()),
    );
    let in_container_config_path = "/app/config";
    environment.insert(
        Yaml::String("CONFIG_PATH".into()),
        Yaml::String(in_container_config_path.into()),
    );
    let in_container_deployment_config_path = "/app/config/deployment.json";
    environment.insert(
        Yaml::String("DEPLOYMEN_CONFIG_PATH".into()),
        Yaml::String(in_container_deployment_config_path.into()),
    );

    if let Ok(feature) = env::var("FEATURE") {
        environment.insert(Yaml::String("FEATURE".into()), Yaml::String(feature));
    }

    Yaml::Hash(environment)
}

fn make_volumes_def(trace_dir: &PathBuf) -> Yaml {
    let in_container_config_path = "/app/config";
    let host_config_dir = trace_dir.to_string_lossy();
    let volume_mapping_config = format!("{}:{}", host_config_dir, in_container_config_path);

    let in_container_deployment_config_path = "/app/config/deployment.json";
    let host_config_path = "./service_configs/deployment.json";
    let volume_mapping_deployment = format!(
        "{}:{}",
        host_config_path, in_container_deployment_config_path
    );

    Yaml::Array(vec![
        Yaml::String(volume_mapping_config),
        Yaml::String(volume_mapping_deployment),
    ])
}

fn make_load_generator_config_yaml(
    trace_dir: &PathBuf,
    deployment: &Deployment,
    replay_path: Option<&Path>,
) -> Result<Yaml> {
    let mut service_def = Hash::new();

    let mut build_def = Hash::new();
    build_def.insert(
        Yaml::String("context".into()),
        Yaml::String(workspace_root().to_string_lossy().to_string()),
    );
    let dockerfile_path = workspace_root().join("apps/mssim/generic-service/Dockerfile.loadgen");
    build_def.insert(
        Yaml::String("dockerfile".into()),
        Yaml::String(dockerfile_path.to_string_lossy().to_string()),
    );

    service_def.insert(Yaml::String("build".into()), Yaml::Hash(build_def));
    service_def.insert(
        Yaml::String("container_name".into()),
        Yaml::String(LOADGEN_SERVICE_NAME.into()),
    );

    let mut environment = Hash::new();

    let frontend_info = deployment
        .services
        .get(&ServiceName::from_string(FRONTEND_SERVICE_NAME.to_string()))
        .ok_or_else(|| anyhow::anyhow!("Frontend service not found in deployment"))?;

    environment.insert(
        Yaml::String("PORT".into()),
        Yaml::String(frontend_info.port.to_string()),
    );

    environment.insert(
        Yaml::String("IP".into()),
        Yaml::String(frontend_info.ip.clone()),
    );

    if let Ok(duration) = env::var("DURATION") {
        environment.insert(
            Yaml::String("DURATION".into()),
            Yaml::String(duration),
        );
    }

    service_def.insert(Yaml::String("environment".into()), Yaml::Hash(environment));

    let host_data_dir = env::var("HOST_TRACE_DIR")
        .unwrap_or_else(|_| trace_dir.clone().to_string_lossy().to_string());

    let mut volumes: Vec<Yaml> = Vec::new();
    let volume_mapping = format!("{}:{}", host_data_dir, LOADGEN_OUTPUT_MOUNT);
    volumes.push(Yaml::String(volume_mapping.into()));
    service_def.insert(Yaml::String("volumes".into()), Yaml::Array(volumes));

    // Add networks (using 'microservice_net' as in the example)
    service_def.insert(
        Yaml::String("networks".into()),
        Yaml::Array(vec![Yaml::String("microservice_net".into())]),
    );

    Ok(Yaml::Hash(service_def))
}

fn run_docker_compose() -> Result<()> {
    info!("Stopping prior docker compose (if any)...");
    let _output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg("-p")
        .arg(PROJECT_NAME) // important: no container is rmed without the project name
        .arg("./docker-compose.yml")
        .arg("down")
        .output()
        .with_context(|| "Failed to execute 'docker-compose down'")?;

    info!("Building and starting Docker compose...");
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg("./docker-compose.yml")
        .arg("-p")
        .arg(PROJECT_NAME) // configure project name to add docker container name prefix
        .arg("up")
        .arg("--build")
        .arg("-d")
        .output()
        .with_context(|| "Failed to execute 'docker-compose up -d, trying with docker compose'")?;

    if output.status.success() {
        info!("Docker Compose started successfully.");
        debug!(
            "Docker Compose output:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if !output.stderr.is_empty() {
            debug!(
                "Docker Compose stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    } else {
        error!("Failed to start Docker Compose.");
        error!("Stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        error!("Stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        Err(anyhow::anyhow!(
            "Failed to start Docker Compose. Check output for details."
        ))
    }
}

fn stop_docker_compose() -> Result<(), anyhow::Error> {
    info!("Stopping Docker Compose.");
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg("./docker-compose.yml")
        .arg("-p")
        .arg(PROJECT_NAME) // important: no container is rmed without the project name
        .arg("down")
        .output()
        .with_context(|| "Failed to execute 'docker-compose down'")?;

    if output.status.success() {
        info!("Docker Compose stopped successfully.");
        debug!(
            "Docker Compose output:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if !output.stderr.is_empty() {
            debug!(
                "Docker Compose stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    } else {
        error!("Failed to stop Docker Compose.");
        error!("Stdout:\n{}", String::from_utf8_lossy(&output.stdout));
        error!("Stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        Err(anyhow::anyhow!(
            "Failed to stop Docker Compose. Check output for details."
        ))
    }
}

pub async fn launch_simulation_from_yaml(
    config: TraceConfig,
    trace_dir: &PathBuf,
    sim_config: SimulatorConfig,
    replay_path: Option<&Path>,
) -> Result<()> {
    // Generate service-specific config files
    let deployment =
        generate_service_configs(config.call_graph.services().into_iter(), &sim_config)?;

    info!("Generated deployment:");
    for d in deployment.services.iter() {
        info!("  Service: {}, Info: {:?}", d.0, d.1);
    }

    // generate docker-compose.yml
    generate_docker_compose(&config, trace_dir, &sim_config, &deployment, replay_path)?;

    // running Docker Compose
    run_docker_compose()?;

    // wait for termination signal (ctrl-c in this case) and then stopping docker compose
    tokio::signal::ctrl_c().await?;
    info!("Received termination signal.");
    stop_docker_compose()?;

    // collect and report output (TODO)
    info!("Collecting and reporting output...");

    Ok(())
}

fn workspace_root() -> PathBuf {
    env!("CARGO_WORKSPACE_DIR").into()
}
