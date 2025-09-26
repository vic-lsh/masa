use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use sim_config::deployment::Deployment;
use sim_config::svc::ServiceName;
use sim_config::trace::TraceConfig;
use std::{collections::HashMap, fs, path::PathBuf, process::Command};
use tracing::{debug, error, info};
use yaml_rust::yaml::Hash;
use yaml_rust::{Yaml, YamlEmitter};

const LOADGEN_SERVICE_NAME: &str = "load_generator";

/// Hard-coded name of the service that is the root of the call graph
///
// TODO: make this configurable
const FRONTEND_SERVICE_NAME: &str = "USER";

const CONTAINER_CPU_LIMIT: usize = 1;

#[allow(dead_code)]
#[derive(Deserialize, Debug, serde::Serialize, Clone)] // Added serde::Serialize and Clone
pub struct ErrorRate {
    #[serde(rename = "distribution_type")]
    // This maps the YAML key 'type' to the Rust field 'distribution_type'
    pub rate_type: String,
    pub parameters: HashMap<String, f64>, // This maps the YAML key 'parameters' to a HashMap
}

pub fn assign_ports(
    service_names: impl Iterator<Item = ServiceName>,
) -> Result<HashMap<ServiceName, u16>> {
    info!("Assigning ports to services.");
    let mut port_assignments = HashMap::new();
    let mut available_ports = (50051..60000).collect::<Vec<u16>>(); // Define a range of ports

    for service_name in service_names {
        if let Some(index) = available_ports.pop() {
            port_assignments.insert(service_name.clone(), index);
            debug!("Assigned port {} to service {}", index, service_name);
        } else {
            error!("Ran out of available ports.");
            return Err(anyhow::anyhow!("Ran out of available ports."));
        }
    }

    info!("Port assignment complete: {:?}", port_assignments);
    Ok(port_assignments)
}

// New function to generate individual config files for each service
pub fn generate_service_configs(port_assignments: &HashMap<ServiceName, u16>) -> Result<()> {
    info!("Generating service-specific configuration files.");
    let config_dir = PathBuf::from("./service_configs"); // Directory to store individual configs

    // Create the config directory if it doesn't exist
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("Failed to create directory: {:?}", config_dir))?;

    // Define the path for the single config file
    let mut service_config_path = config_dir.clone();
    let output_filename = "deployment.json";
    service_config_path.push(output_filename);

    let deployment = make_deployment_config(port_assignments);

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

fn make_deployment_config(port_assignments: &HashMap<ServiceName, u16>) -> Deployment {
    let mut services = HashMap::new();

    for (service_name, port) in port_assignments {
        println!("Service: {}, Port: {}", service_name, port);

        services.insert(
            service_name.clone(),
            sim_config::deployment::ServiceDiscoveryInfo {
                // in our docker config, service name is the ip
                ip: service_name.to_string(),
                port: *port,
            },
        );
    }

    Deployment { services }
}

pub fn generate_docker_compose(
    config: &TraceConfig,
    ports: &HashMap<ServiceName, u16>,
    trace_dir: &PathBuf,
) -> Result<()> {
    info!("Generating docker-compose.yml file.");
    let mut doc_hash = Hash::new();

    doc_hash.insert(Yaml::String("version".into()), Yaml::String("3".into()));

    let mut services = Hash::new();
    for service_name in &config.call_graph.services() {
        let svc_port = ports
            .get(service_name)
            .ok_or_else(|| anyhow::anyhow!("Port not assigned for service: {}", service_name))?;

        let mut service_def = Hash::new();

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
        // Pass the port as a build argument (still useful for EXPOSE in Dockerfile)
        let mut build_args = Hash::new();
        build_args.insert(
            Yaml::String("SERVICE_CONTAINER_PORT".into()),
            Yaml::String(svc_port.to_string()),
        );
        build_def.insert(Yaml::String("args".into()), Yaml::Hash(build_args));

        service_def.insert(Yaml::String("build".into()), Yaml::Hash(build_def));
        service_def.insert(
            Yaml::String("container_name".into()),
            Yaml::String(service_name.clone().into()),
        );

        let mut deploy_def = Hash::new();
        let mut resources_def = Hash::new();
        let mut limits_def = Hash::new();

        // TODO: make these configurable
        limits_def.insert(
            Yaml::String("cpus".into()),
            Yaml::String(CONTAINER_CPU_LIMIT.to_string()),
        );
        // TODO: do we need memory limits?
        // limits_def.insert(
        //     Yaml::String("memory".into()),
        //     Yaml::String("512M".into()), // Limit to 512MB memory
        // );
        resources_def.insert(Yaml::String("limits".into()), Yaml::Hash(limits_def));
        deploy_def.insert(Yaml::String("resources".into()), Yaml::Hash(resources_def));
        service_def.insert(Yaml::String("deploy".into()), Yaml::Hash(deploy_def));

        if let Some(&host_port) = ports.get(service_name) {
            let ports_mapping = format!("{}:{}", host_port, svc_port);
            service_def.insert(
                Yaml::String("ports".into()),
                Yaml::Array(vec![Yaml::String(ports_mapping)]),
            );
        } else {
            error!("Port not assigned for service: {}", service_name);
            return Err(anyhow::anyhow!(
                "Port not assigned for service: {}",
                service_name
            ));
        }

        let mut environment = Hash::new();
        // Add the SERVICE_NAME environment variable
        environment.insert(
            Yaml::String("SERVICE_NAME".into()),
            Yaml::String(service_name.into()),
        );

        // Add the SERVICE_PORT environment variable
        environment.insert(
            Yaml::String("SERVICE_PORT".into()),
            Yaml::String(svc_port.to_string()),
        );

        // Define the path where the config file will be mounted INSIDE the container
        let in_container_config_path = "/app/config"; // Example path inside the container
        environment.insert(
            Yaml::String("CONFIG_PATH".into()),
            Yaml::String(in_container_config_path.into()),
        );
        let in_container_deployment_config_path = "/app/config/deployment.json"; // Example path inside the container
        environment.insert(
            Yaml::String("DEPLOYMEN_CONFIG_PATH".into()),
            Yaml::String(in_container_deployment_config_path.into()),
        );

        service_def.insert(Yaml::String("environment".into()), Yaml::Hash(environment));

        // Configure volumes to mount the service-specific config file
        let mut volumes: Vec<Yaml> = Vec::new();

        // add config volume
        // let host_config_dir = workspace_root()
        //     .join("./trace-analysis/golden/S_32048416/")
        //     .to_string_lossy()
        //     .into_owned();
        let host_config_dir = trace_dir.to_string_lossy().into_owned();
        let volume_mapping = format!("{}:{}", host_config_dir, in_container_config_path);
        volumes.push(Yaml::String(volume_mapping.into()));

        // Add deployment config volume
        let host_config_path = format!("./service_configs/deployment.json");
        let volume_mapping = format!(
            "{}:{}",
            host_config_path, in_container_deployment_config_path
        );
        volumes.push(Yaml::String(volume_mapping.into()));

        service_def.insert(Yaml::String("volumes".into()), Yaml::Array(volumes));

        // Add networks (using 'microservice_net' as in the example)
        service_def.insert(
            Yaml::String("networks".into()),
            Yaml::Array(vec![Yaml::String("microservice_net".into())]),
        );

        // depends_on logic can be adjusted or removed based on whether Docker Compose startup order is critical
        // Based on previous errors and the new config method, removing automatic depends_on from calls might be necessary
        // or implementing more sophisticated dependency analysis.
        // Keeping it commented out for now as per previous discussion.
        /*
        let mut dependencies: Vec<Yaml> = Vec::new();
         // ... dependency logic ...
        if !dependencies.is_empty() {
             service_def.insert(Yaml::String("depends_on".into()), Yaml::Array(dependencies));
        } else {
              service_def.insert(Yaml::String("depends_on".into()), Yaml::Null);
        }
        */

        services.insert(
            Yaml::String(service_name.to_string()),
            Yaml::Hash(service_def),
        );
    }

    let frontend_port = *ports
        .get(&ServiceName::from_string(FRONTEND_SERVICE_NAME.to_string()))
        .ok_or_else(|| anyhow::anyhow!("Port not assigned for frontend service"))?;

    let loadgen_config = make_load_generator_config(frontend_port)?;
    // Add load generator
    services.insert(
        Yaml::String(LOADGEN_SERVICE_NAME.into()),
        Yaml::Hash(loadgen_config),
    );

    doc_hash.insert(Yaml::String("services".into()), Yaml::Hash(services));

    // Add the networks definition at the top level
    let mut networks_def = Hash::new();
    let mut microservice_net_def = Hash::new();
    microservice_net_def.insert(Yaml::String("driver".into()), Yaml::String("bridge".into()));
    networks_def.insert(
        Yaml::String("microservice_net".into()),
        Yaml::Hash(microservice_net_def),
    );
    doc_hash.insert(Yaml::String("networks".into()), Yaml::Hash(networks_def));

    let doc = Yaml::Hash(doc_hash);

    let mut output_string = String::new();
    let mut emitter = YamlEmitter::new(&mut output_string);
    emitter.dump(&doc).unwrap();

    let mut compose_path = PathBuf::from(".");
    compose_path.push("docker-compose.yml");

    fs::write(&compose_path, output_string).with_context(|| {
        format!(
            "Failed to write docker-compose.yml file to {:?}",
            compose_path
        )
    })?;

    info!("docker-compose.yml file generated successfully.");

    Ok(())
}

fn make_load_generator_config(frontend_port: u16) -> Result<Hash> {
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
    environment.insert(
        Yaml::String("PORT".into()),
        Yaml::String(frontend_port.to_string()),
    );
    environment.insert(Yaml::String("IP".into()), Yaml::String("user".into()));

    service_def.insert(Yaml::String("environment".into()), Yaml::Hash(environment));

    // Add networks (using 'microservice_net' as in the example)
    service_def.insert(
        Yaml::String("networks".into()),
        Yaml::Array(vec![Yaml::String("microservice_net".into())]),
    );

    Ok(service_def)
}

fn run_docker_compose() -> Result<()> {
    info!("Starting Docker Compose.");
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg("./docker-compose.yml")
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

pub async fn launch_simulation_from_yaml(config: TraceConfig, trace_dir: &PathBuf) -> Result<()> {
    // assign ports
    let port_assignments = assign_ports(config.call_graph.services().into_iter())?;
    info!("Port assignments: {:?}", port_assignments);

    // Generate service-specific config files
    generate_service_configs(&port_assignments)?;

    // generate docker-compose.yml
    generate_docker_compose(&config, &port_assignments, trace_dir)?;

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
