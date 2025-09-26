use anyhow::{Context, Result, anyhow};
use sim_config::svc::ServiceName;
use sim_config::trace::TraceConfig;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};
use tracing::{debug, error, info};
use yaml_rust::yaml::Hash;
use yaml_rust::{Yaml, YamlEmitter};

const LOADGEN_SERVICE_NAME: &str = "load_generator";

pub fn generate_compose(
    config: &TraceConfig,
    ports: &HashMap<ServiceName, u16>,
    trace_dir: &Path,
    service_config_dir: &Path,
    workspace_root: &Path,
    container_cpu_limit: usize,
) -> Result<()> {
    info!("Generating docker-compose.yml file.");

    let mut doc_hash = Hash::new();
    doc_hash.insert(Yaml::String("version".into()), Yaml::String("3".into()));

    let service_names = config.call_graph.services();
    let mut services = Hash::new();

    for service_name in service_names.iter() {
        let svc_port = ports
            .get(service_name)
            .with_context(|| format!("Port not assigned for service: {}", service_name))?;

        let mut service_def = Hash::new();

        let mut build_def = Hash::new();
        build_def.insert(
            Yaml::String("context".into()),
            Yaml::String(workspace_root.to_string_lossy().to_string()),
        );
        let dockerfile_path = workspace_root.join("apps/mssim/generic-service/Dockerfile");
        build_def.insert(
            Yaml::String("dockerfile".into()),
            Yaml::String(dockerfile_path.to_string_lossy().to_string()),
        );

        let mut build_args = Hash::new();
        build_args.insert(
            Yaml::String("SERVICE_CONTAINER_PORT".into()),
            Yaml::String(svc_port.to_string()),
        );
        build_def.insert(Yaml::String("args".into()), Yaml::Hash(build_args));

        service_def.insert(Yaml::String("build".into()), Yaml::Hash(build_def));
        service_def.insert(
            Yaml::String("container_name".into()),
            Yaml::String(service_name.to_string()),
        );

        let mut deploy_def = Hash::new();
        let mut resources_def = Hash::new();
        let mut limits_def = Hash::new();
        limits_def.insert(
            Yaml::String("cpus".into()),
            Yaml::String(container_cpu_limit.to_string()),
        );
        resources_def.insert(Yaml::String("limits".into()), Yaml::Hash(limits_def));
        deploy_def.insert(Yaml::String("resources".into()), Yaml::Hash(resources_def));
        service_def.insert(Yaml::String("deploy".into()), Yaml::Hash(deploy_def));

        let host_port = ports
            .get(service_name)
            .copied()
            .ok_or_else(|| anyhow!("Port not assigned for service: {}", service_name))?;
        let ports_mapping = format!("{}:{}", host_port, svc_port);
        service_def.insert(
            Yaml::String("ports".into()),
            Yaml::Array(vec![Yaml::String(ports_mapping)]),
        );

        let mut environment = Hash::new();
        environment.insert(
            Yaml::String("SERVICE_NAME".into()),
            Yaml::String(service_name.to_string()),
        );
        environment.insert(
            Yaml::String("SERVICE_PORT".into()),
            Yaml::String(svc_port.to_string()),
        );
        environment.insert(
            Yaml::String("CONFIG_PATH".into()),
            Yaml::String("/app/config".into()),
        );
        environment.insert(
            Yaml::String("DEPLOYMEN_CONFIG_PATH".into()),
            Yaml::String("/app/config/deployment.json".into()),
        );
        service_def.insert(Yaml::String("environment".into()), Yaml::Hash(environment));

        let mut volumes: Vec<Yaml> = Vec::new();
        let volume_mapping = format!("{}:{}", trace_dir.to_string_lossy(), "/app/config");
        volumes.push(Yaml::String(volume_mapping));

        let deployment_config_path = service_config_dir.join("deployment.json");
        let volume_mapping = format!(
            "{}:{}",
            deployment_config_path.to_string_lossy(),
            "/app/config/deployment.json"
        );
        volumes.push(Yaml::String(volume_mapping));

        service_def.insert(Yaml::String("volumes".into()), Yaml::Array(volumes));
        service_def.insert(
            Yaml::String("networks".into()),
            Yaml::Array(vec![Yaml::String("microservice_net".into())]),
        );

        services.insert(
            Yaml::String(service_name.to_string()),
            Yaml::Hash(service_def),
        );
    }

    let frontend_service_name = ServiceName::from_string("USER".to_string());
    let frontend_port = ports
        .get(&frontend_service_name)
        .copied()
        .with_context(|| "Port not assigned for frontend service")?;

    let loadgen_config = make_load_generator_config(frontend_port, workspace_root)?;
    services.insert(
        Yaml::String(LOADGEN_SERVICE_NAME.into()),
        Yaml::Hash(loadgen_config),
    );

    doc_hash.insert(Yaml::String("services".into()), Yaml::Hash(services));

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

    let compose_path = PathBuf::from("docker-compose.yml");
    std::fs::write(&compose_path, output_string).with_context(|| {
        format!(
            "Failed to write docker-compose.yml file to {:?}",
            compose_path
        )
    })?;

    info!("docker-compose.yml file generated successfully.");
    Ok(())
}

pub fn run() -> Result<()> {
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
            "Docker Compose stdout:\n{}",
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
        Err(anyhow!(
            "Failed to start Docker Compose. Check output for details."
        ))
    }
}

pub fn stop() -> Result<()> {
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
            "Docker Compose stdout:\n{}",
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
        Err(anyhow!(
            "Failed to stop Docker Compose. Check output for details."
        ))
    }
}

fn make_load_generator_config(frontend_port: u16, workspace_root: &Path) -> Result<Hash> {
    let mut service_def = Hash::new();

    let mut build_def = Hash::new();
    build_def.insert(
        Yaml::String("context".into()),
        Yaml::String(workspace_root.to_string_lossy().to_string()),
    );
    let dockerfile_path = workspace_root.join("apps/mssim/generic-service/Dockerfile.loadgen");
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
    service_def.insert(
        Yaml::String("networks".into()),
        Yaml::Array(vec![Yaml::String("microservice_net".into())]),
    );

    Ok(service_def)
}
