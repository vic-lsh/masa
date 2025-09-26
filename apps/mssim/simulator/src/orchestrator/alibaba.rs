use super::Backend;
use anyhow::{Context, Result, anyhow};
use serde::Deserialize;
use serde_json::json;
use sim_config::deployment::{Deployment, ServiceDiscoveryInfo};
use sim_config::svc::ServiceName;
use sim_config::trace::TraceConfig;
use sim_config::{PROJECT_NAME, SimulatorConfig};
use std::{
    collections::HashMap,
    fs,
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
const LOADGEN_TRACE_MOUNT: &str = "/trace-data";
const CONTAINER_CPU_LIMIT: usize = 1;
const CONTAINER_MEM_LIMIT: &str = "512MB";

const DEFAULT_SVC_PORT: u16 = 50051;

const GENERIC_SERVICE_IMAGE_TAG: &str = "mssim/generic-service:latest";
const LOAD_GENERATOR_IMAGE_TAG: &str = "mssim/load-generator:latest";
const K8S_MANIFEST_FILENAME: &str = "k8s-manifest.yaml";

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

#[derive(Debug)]
struct K8sImages {
    generic_service: String,
    load_generator: String,
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

    let trace_dir_canon = trace_dir
        .canonicalize()
        .with_context(|| format!("Failed to resolve trace directory {:?}", trace_dir))?;
    let host_trace_dir = trace_dir_canon.to_string_lossy().into_owned();

    let replay_env = if let Some(override_path) = replay_path {
        let resolved = if override_path.is_absolute() {
            override_path.to_path_buf()
        } else {
            trace_dir_canon.join(override_path)
        };

        let replay_canon = resolved
            .canonicalize()
            .with_context(|| format!("Failed to resolve replay trace path {:?}", resolved))?;

        let relative = replay_canon
            .strip_prefix(&trace_dir_canon)
            .with_context(|| {
                anyhow!(
                    "Replay trace path {:?} must be located under {:?}",
                    replay_canon,
                    trace_dir_canon
                )
            })?;

        Some(
            Path::new(LOADGEN_TRACE_MOUNT)
                .join(relative)
                .to_string_lossy()
                .into_owned(),
        )
    } else {
        None
    };

    if let Some(replay_path) = replay_env {
        environment.insert(
            Yaml::String("REPLAY_TRACE_PATH".into()),
            Yaml::String(replay_path),
        );
    }
    environment.insert(
        Yaml::String("QUEUE_LATENCY_OUTPUT_DIR".into()),
        Yaml::String(LOADGEN_TRACE_MOUNT.into()),
    );

    service_def.insert(Yaml::String("environment".into()), Yaml::Hash(environment));

    let mut volumes: Vec<Yaml> = Vec::new();
    let volume_mapping = format!("{}:{}", host_trace_dir, LOADGEN_TRACE_MOUNT);
    volumes.push(Yaml::String(volume_mapping.into()));
    service_def.insert(Yaml::String("volumes".into()), Yaml::Array(volumes));

    // Add networks (using 'microservice_net' as in the example)
    service_def.insert(
        Yaml::String("networks".into()),
        Yaml::Array(vec![Yaml::String("microservice_net".into())]),
    );

    Ok(Yaml::Hash(service_def))
}

fn build_k8s_images() -> Result<K8sImages> {
    info!("Building Docker images for Kubernetes backend.");

    let generic_dockerfile = workspace_root().join("apps/mssim/generic-service/Dockerfile");
    build_docker_image(
        GENERIC_SERVICE_IMAGE_TAG,
        &generic_dockerfile,
        &[("SERVICE_CONTAINER_PORT", "50051".to_string())],
    )?;

    let loadgen_dockerfile = workspace_root().join("apps/mssim/generic-service/Dockerfile.loadgen");
    build_docker_image(LOAD_GENERATOR_IMAGE_TAG, &loadgen_dockerfile, &[])?;

    Ok(K8sImages {
        generic_service: GENERIC_SERVICE_IMAGE_TAG.to_string(),
        load_generator: LOAD_GENERATOR_IMAGE_TAG.to_string(),
    })
}

fn build_docker_image(tag: &str, dockerfile: &Path, build_args: &[(&str, String)]) -> Result<()> {
    info!("Building Docker image {} using {:?}", tag, dockerfile);

    let mut cmd = Command::new("docker");
    cmd.arg("build")
        .arg("-t")
        .arg(tag)
        .arg("-f")
        .arg(dockerfile);

    for (key, value) in build_args {
        cmd.arg("--build-arg").arg(format!("{}={}", key, value));
    }

    cmd.arg(workspace_root());

    let output = cmd
        .output()
        .with_context(|| format!("Failed to execute docker build for {}", tag))?;

    if output.status.success() {
        debug!(
            "docker build ({}) stdout:\n{}",
            tag,
            String::from_utf8_lossy(&output.stdout)
        );
        if !output.stderr.is_empty() {
            debug!(
                "docker build ({}) stderr:\n{}",
                tag,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    } else {
        error!(
            "docker build ({}) stdout:\n{}",
            tag,
            String::from_utf8_lossy(&output.stdout)
        );
        error!(
            "docker build ({}) stderr:\n{}",
            tag,
            String::from_utf8_lossy(&output.stderr)
        );
        Err(anyhow!("Failed to build Docker image {}", tag))
    }
}

fn generate_k8s_manifest(
    config: &TraceConfig,
    deployment: &Deployment,
    trace_dir: &PathBuf,
    images: &K8sImages,
) -> Result<PathBuf> {
    info!("Generating Kubernetes manifest file.");

    let trace_dir_abs = trace_dir.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize trace directory for Kubernetes manifest: {:?}",
            trace_dir
        )
    })?;

    let service_config_dir = PathBuf::from("./service_configs");
    let service_config_dir_abs = service_config_dir.canonicalize().with_context(|| {
        format!(
            "Failed to canonicalize service config directory: {:?}",
            service_config_dir
        )
    })?;

    let trace_dir_str = trace_dir_abs.to_string_lossy().to_string();
    let service_config_dir_str = service_config_dir_abs.to_string_lossy().to_string();

    let mut manifest_docs: Vec<serde_json::Value> = Vec::new();

    for service_name in &config.call_graph.services() {
        let svc_info = deployment
            .services
            .get(service_name)
            .ok_or_else(|| anyhow::anyhow!("Service not found in deployment: {}", service_name))?;
        let svc_port = svc_info.port;
        let svc_name = service_name.to_string();
        let labels = json!({"app": "mssim", "service": svc_name.clone()});
        let svc_port_str = svc_port.to_string();

        let deployment = json!({
            "apiVersion": "apps/v1",
            "kind": "Deployment",
            "metadata": {
                "name": svc_name.clone(),
                "labels": labels.clone()
            },
            "spec": {
                "replicas": 1,
                "selector": {"matchLabels": labels.clone()},
                "template": {
                    "metadata": {"labels": labels.clone()},
                    "spec": {
                        "containers": [{
                            "name": svc_name.clone(),
                            "image": images.generic_service.clone(),
                            "imagePullPolicy": "Never",
                            "env": [
                                {"name": "SERVICE_NAME", "value": svc_name.clone()},
                                {"name": "SERVICE_PORT", "value": svc_port_str},
                                {"name": "CONFIG_PATH", "value": "/app/config"},
                                {"name": "DEPLOYMEN_CONFIG_PATH", "value": "/app/config/deployment.json"}
                            ],
                            "ports": [{"containerPort": svc_port}],
                            "resources": {
                                "limits": {"cpu": CONTAINER_CPU_LIMIT.to_string()}
                            },
                            "volumeMounts": [
                                {"name": "trace-config", "mountPath": "/app/config"},
                                {
                                    "name": "deployment-config",
                                    "mountPath": "/app/config/deployment.json",
                                    "subPath": "deployment.json"
                                }
                            ]
                        }],
                        "volumes": [
                            {
                                "name": "trace-config",
                                "hostPath": {
                                    "path": trace_dir_str.clone(),
                                    "type": "Directory"
                                }
                            },
                            {
                                "name": "deployment-config",
                                "hostPath": {
                                    "path": service_config_dir_str.clone(),
                                    "type": "Directory"
                                }
                            }
                        ]
                    }
                }
            }
        });

        let service = json!({
            "apiVersion": "v1",
            "kind": "Service",
            "metadata": {
                "name": svc_name.clone(),
                "labels": labels.clone()
            },
            "spec": {
                "selector": labels,
                "ports": [{
                    "name": "grpc",
                    "port": svc_port,
                    "targetPort": svc_port
                }]
            }
        });

        manifest_docs.push(deployment);
        manifest_docs.push(service);
    }

    let frontend_service_name = ServiceName::from_string(FRONTEND_SERVICE_NAME.to_string());
    let frontend_info = deployment
        .services
        .get(&frontend_service_name)
        .ok_or_else(|| anyhow::anyhow!("Frontend service not found in deployment"))?;
    let frontend_port = frontend_info.port;
    let frontend_dns = frontend_info.ip.clone();

    let loadgen_labels = json!({"app": "mssim", "service": LOADGEN_SERVICE_NAME});
    let loadgen_deployment = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": LOADGEN_SERVICE_NAME,
            "labels": loadgen_labels
        },
        "spec": {
            "replicas": 1,
            "selector": {"matchLabels": loadgen_labels},
            "template": {
                "metadata": {"labels": loadgen_labels},
                "spec": {
                    "containers": [{
                        "name": LOADGEN_SERVICE_NAME,
                        "image": images.load_generator.clone(),
                        "imagePullPolicy": "Never",
                        "env": [
                            {"name": "PORT", "value": frontend_port.to_string()},
                            {"name": "IP", "value": frontend_dns}
                        ]
                    }]
                }
            }
        }
    });

    manifest_docs.push(loadgen_deployment);

    let mut manifest_contents = String::new();
    for doc in manifest_docs {
        let yaml_doc = serde_yaml::to_string(&doc)
            .with_context(|| "Failed to serialize Kubernetes manifest segment")?;
        if !manifest_contents.is_empty() {
            manifest_contents.push_str("---\n");
        }
        manifest_contents.push_str(&yaml_doc);
        if !manifest_contents.ends_with('\n') {
            manifest_contents.push('\n');
        }
    }

    let manifest_path = PathBuf::from(K8S_MANIFEST_FILENAME);
    fs::write(&manifest_path, manifest_contents).with_context(|| {
        format!(
            "Failed to write Kubernetes manifest file to {:?}",
            manifest_path
        )
    })?;

    info!("Kubernetes manifest generated at {:?}", manifest_path);
    Ok(manifest_path)
}

fn kubectl_apply(manifest_path: &Path) -> Result<()> {
    info!("Applying Kubernetes manifest {:?}", manifest_path);
    let output = Command::new("kubectl")
        .arg("apply")
        .arg("-f")
        .arg(manifest_path)
        .output()
        .with_context(|| format!("Failed to execute kubectl apply for {:?}", manifest_path))?;

    if output.status.success() {
        debug!(
            "kubectl apply stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if !output.stderr.is_empty() {
            debug!(
                "kubectl apply stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    } else {
        error!(
            "kubectl apply stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        error!(
            "kubectl apply stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Err(anyhow!(
            "Failed to apply Kubernetes manifest. Check kubectl output for details."
        ))
    }
}

fn kubectl_delete(manifest_path: &Path) -> Result<()> {
    info!(
        "Deleting Kubernetes resources defined in {:?}",
        manifest_path
    );
    let output = Command::new("kubectl")
        .arg("delete")
        .arg("-f")
        .arg(manifest_path)
        .output()
        .with_context(|| format!("Failed to execute kubectl delete for {:?}", manifest_path))?;

    if output.status.success() {
        debug!(
            "kubectl delete stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        if !output.stderr.is_empty() {
            debug!(
                "kubectl delete stderr:\n{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    } else {
        error!(
            "kubectl delete stdout:\n{}",
            String::from_utf8_lossy(&output.stdout)
        );
        error!(
            "kubectl delete stderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Err(anyhow!(
            "Failed to delete Kubernetes resources. Check kubectl output for details."
        ))
    }
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
    backend: Backend,
) -> Result<()> {
    // Generate service-specific config files
    let deployment =
        generate_service_configs(config.call_graph.services().into_iter(), &sim_config)?;

    info!("Generated deployment:");
    for d in deployment.services.iter() {
        info!("  Service: {}, Info: {:?}", d.0, d.1);
    }

    match backend {
        Backend::DockerCompose => {
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
        Backend::Kubernetes => {
            let images = build_k8s_images()?;
            let manifest_path =
                generate_k8s_manifest(&config, &deployment, trace_dir, &images)?;

            kubectl_apply(&manifest_path)?;

            tokio::signal::ctrl_c().await?;
            info!("Received termination signal.");

            kubectl_delete(&manifest_path)?;

            info!("Collecting and reporting output...");
            Ok(())
        }
    }
}

fn workspace_root() -> PathBuf {
    env!("CARGO_WORKSPACE_DIR").into()
}
