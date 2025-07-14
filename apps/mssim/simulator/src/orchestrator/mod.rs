use anyhow::{Context, Result};
use serde::Deserialize;
use std::{collections::HashMap, fs, path::PathBuf, process::Command};
use tracing::{debug, error, info};
use yaml_rust::yaml::Hash;
use yaml_rust::{Yaml, YamlEmitter};

use crate::parser::{MethodConfig, ServiceConfig, SimulatorConfig};

#[derive(Deserialize, Debug, serde::Serialize, Clone)] // Added serde::Serialize and Clone
pub struct ErrorRate {
    #[serde(rename = "distribution_type")]
    // This maps the YAML key 'type' to the Rust field 'distribution_type'
    pub rate_type: String,
    pub parameters: HashMap<String, f64>, // This maps the YAML key 'parameters' to a HashMap
}

pub fn assign_ports(services: &HashMap<String, ServiceConfig>) -> Result<HashMap<String, u16>> {
    info!("Assigning ports to services.");
    let mut port_assignments = HashMap::new();
    let mut available_ports = (50051..60000).collect::<Vec<u16>>(); // Define a range of ports

    for service_name in services.keys() {
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
pub fn generate_service_configs(config: &SimulatorConfig) -> Result<()> {
    info!("Generating service-specific configuration files.");
    let config_dir = PathBuf::from("./service_configs"); // Directory to store individual configs

    // Create the config directory if it doesn't exist
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("Failed to create directory: {:?}", config_dir))?;

    // Define the path for the single config file
    let mut service_config_path = config_dir.clone();
    let output_filename = "config.json";
    service_config_path.push(output_filename);

    // New struct that matches the format expected *inside* the service name key in the output JSON
    #[derive(serde::Serialize, Clone)] // Only needs Serialize and Clone for generating the output file
    pub struct GenericServiceServiceConfig {
        pub ip: String,                             // Matches "ip" in example JSON
        pub port: String,                           // Matches "port" in example JSON (as String)
        pub methods: HashMap<String, MethodConfig>, // Matches "methods" in example JSON (MethodConfig already has derives)
    }

    // making hashmap to store the configs for each service
    let mut all_service_configs: HashMap<String, GenericServiceServiceConfig> = HashMap::new();

    // populating the hashmap
    for (service_name, service_config) in &config.services {
        // Create the config object for this service in the desired output format
        let generic_service_config = GenericServiceServiceConfig {
            ip: service_name.clone(),
            port: service_config.port.to_string(),
            methods: service_config.methods.clone(),
        };

        // Insert the service's config into the map, using the service name as the key
        all_service_configs.insert(service_name.clone(), generic_service_config);
    }

    // Serialize the entire map containing all service configs
    let config_json = serde_json::to_string_pretty(&all_service_configs)
        .with_context(|| "Failed to serialize all service configurations")?;

    // Write the entire config to the single file
    fs::write(&service_config_path, config_json).with_context(|| {
        format!(
            "Failed to write the single config file to {:?}",
            service_config_path
        )
    })?;

    info!(
        "Created config file containing all service configurations at {:?}",
        service_config_path
    );

    Ok(())
}

pub fn generate_docker_compose(
    config: &SimulatorConfig,
    ports: &HashMap<String, u16>,
) -> Result<()> {
    info!("Generating docker-compose.yml file.");
    let mut doc_hash = Hash::new();

    doc_hash.insert(Yaml::String("version".into()), Yaml::String("3".into()));

    let mut services = Hash::new();
    for (service_name, service_config) in &config.services {
        let mut service_def = Hash::new();

        let mut build_def = Hash::new();
        build_def.insert(
            Yaml::String("context".into()),
            Yaml::String("../generic-service".into()),
        );
        build_def.insert(
            Yaml::String("dockerfile".into()),
            Yaml::String("Dockerfile".into()),
        );
        // Pass the port as a build argument (still useful for EXPOSE in Dockerfile)
        let mut build_args = Hash::new();
        build_args.insert(
            Yaml::String("SERVICE_CONTAINER_PORT".into()),
            Yaml::String(service_config.port.to_string()),
        );
        build_def.insert(Yaml::String("args".into()), Yaml::Hash(build_args));

        service_def.insert(Yaml::String("build".into()), Yaml::Hash(build_def));
        service_def.insert(
            Yaml::String("container_name".into()),
            Yaml::String(service_name.clone().into()),
        );

        if let Some(&host_port) = ports.get(service_name) {
            let ports_mapping = format!("{}:{}", host_port, service_config.port);
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
            Yaml::String(service_name.clone().into()),
        );

        // Add the SERVICE_PORT environment variable
        environment.insert(
            Yaml::String("SERVICE_PORT".into()),
            Yaml::String(service_config.port.to_string()),
        );

        // Define the path where the config file will be mounted INSIDE the container
        let container_config_path = "/app/config.json"; // Example path inside the container
        environment.insert(
            Yaml::String("CONFIG_PATH".into()),
            Yaml::String(container_config_path.into()),
        );

        service_def.insert(Yaml::String("environment".into()), Yaml::Hash(environment));

        // Configure volumes to mount the service-specific config file
        let mut volumes: Vec<Yaml> = Vec::new();
        // Path on the host: ./service_configs/config.json
        let host_config_path = format!("./service_configs/config.json");
        // Mount point inside the container: /app/config.json (matches CONFIG_PATH)
        let volume_mapping = format!("{}:{}", host_config_path, container_config_path);
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

        services.insert(Yaml::String(service_name.clone()), Yaml::Hash(service_def));
    }

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

fn run_docker_compose() -> Result<()> {
    info!("Starting Docker Compose.");
    let output = Command::new("docker")
        .arg("compose")
        .arg("-f")
        .arg("./docker-compose.yml")
        .arg("up")
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

pub async fn launch_simulation_from_yaml(config: SimulatorConfig) -> Result<()> {
    // assign ports
    let port_assignments = assign_ports(&config.services)?;
    info!("Port assignments: {:?}", port_assignments);

    // Generate service-specific config files
    generate_service_configs(&config)?;

    // generate docker-compose.yml
    generate_docker_compose(&config, &port_assignments)?;

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
