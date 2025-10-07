use anyhow::{Context, Result, anyhow};
use serde_json::json;
use sim_config::svc::ServiceName;
use sim_config::trace::TraceConfig;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::Command,
};
use tracing::{debug, error, info};

const LOADGEN_SERVICE_NAME: &str = "load-generator";
const GENERIC_SERVICE_IMAGE_TAG: &str = "mssim/generic-service:latest";
const LOAD_GENERATOR_IMAGE_TAG: &str = "mssim/load-generator:latest";
const K8S_MANIFEST_FILENAME: &str = "k8s-manifest.yaml";

#[derive(Debug)]
pub struct Images {
    pub generic_service: String,
    pub load_generator: String,
}

pub fn build_images(workspace_root: &Path) -> Result<Images> {
    info!("Building Docker images for Kubernetes backend using minikube docker.");

    let generic_dockerfile = workspace_root.join("apps/mssim/generic-service/Dockerfile");
    build_docker_image_minikube(
        GENERIC_SERVICE_IMAGE_TAG,
        &generic_dockerfile,
        &[("SERVICE_CONTAINER_PORT", "50051".to_string())],
        workspace_root,
    )?;

    let loadgen_dockerfile = workspace_root.join("apps/mssim/generic-service/Dockerfile.loadgen");
    build_docker_image_minikube(
        LOAD_GENERATOR_IMAGE_TAG,
        &loadgen_dockerfile,
        &[],
        workspace_root,
    )?;

    Ok(Images {
        generic_service: GENERIC_SERVICE_IMAGE_TAG.to_string(),
        load_generator: LOAD_GENERATOR_IMAGE_TAG.to_string(),
    })
}

pub fn generate_manifest(
    config: &TraceConfig,
    ports: &HashMap<ServiceName, u16>,
    trace_dir: &Path,
    service_config_dir: &Path,
    images: &Images,
    container_cpu_limit: usize,
) -> Result<PathBuf> {
    info!("Generating Kubernetes manifest file.");

    // For Minikube, use the mounted paths instead of host paths
    // These directories must be mounted using: minikube mount <host-path>:<mount-path>
    let trace_dir_str = "/trace-data".to_string();
    let service_config_dir_str = "/service-configs".to_string();

    let mut manifest_docs: Vec<serde_json::Value> = Vec::new();
    let service_names = config.call_graph.services();

    for service_name in service_names.iter() {
        let svc_port = ports
            .get(service_name)
            .copied()
            .with_context(|| format!("Port not assigned for service: {}", service_name))?;

        let svc_name = service_name.to_string();
        let labels = json!({"app": "mssim", "service": svc_name.clone()});

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
                                {"name": "SERVICE_PORT", "value": svc_port.to_string()},
                                {"name": "CONFIG_PATH", "value": "/app/config"},
                                {"name": "DEPLOYMEN_CONFIG_PATH", "value": "/app/config/deployment.json"}
                            ],
                            "ports": [{"containerPort": svc_port}],
                            "resources": {
                                "limits": {"cpu": format!("{}m", container_cpu_limit * 100)}
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

    let frontend_service_name = ServiceName::from_string("USER".to_string());
    let frontend_port = ports
        .get(&frontend_service_name)
        .copied()
        .with_context(|| {
            format!(
                "Port not assigned for frontend service {}",
                frontend_service_name
            )
        })?;
    let frontend_dns = frontend_service_name.to_string();

    let loadgen_labels = json!({"app": "mssim", "service": LOADGEN_SERVICE_NAME});
    let loadgen_deployment = json!({
        "apiVersion": "apps/v1",
        "kind": "Deployment",
        "metadata": {
            "name": LOADGEN_SERVICE_NAME,
            "labels": loadgen_labels.clone()
        },
        "spec": {
            "replicas": 1,
            "selector": {"matchLabels": loadgen_labels.clone()},
            "template": {
                "metadata": {"labels": loadgen_labels.clone()},
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
    std::fs::write(&manifest_path, manifest_contents).with_context(|| {
        format!(
            "Failed to write Kubernetes manifest file to {:?}",
            manifest_path
        )
    })?;

    info!("Kubernetes manifest generated at {:?}", manifest_path);
    Ok(manifest_path)
}

pub fn apply(manifest_path: &Path) -> Result<()> {
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

pub fn delete(manifest_path: &Path) -> Result<()> {
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

fn build_docker_image_minikube(
    tag: &str,
    dockerfile: &Path,
    build_args: &[(&str, String)],
    workspace_root: &Path,
) -> Result<()> {
    info!("Building Docker image {} for minikube using {:?}", tag, dockerfile);

    // Get minikube's docker environment variables
    let docker_env_output = Command::new("minikube")
        .arg("docker-env")
        .arg("--shell=none")
        .output()
        .with_context(|| "Failed to get minikube docker-env")?;

    if !docker_env_output.status.success() {
        return Err(anyhow!(
            "Failed to get minikube docker environment: {}",
            String::from_utf8_lossy(&docker_env_output.stderr)
        ));
    }

    // Parse environment variables from minikube docker-env output
    let env_output = String::from_utf8_lossy(&docker_env_output.stdout);
    let mut docker_host = None;
    let mut docker_cert_path = None;
    let mut docker_tls_verify = None;

    for line in env_output.lines() {
        if let Some(value) = line.strip_prefix("DOCKER_HOST=") {
            docker_host = Some(value.trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("DOCKER_CERT_PATH=") {
            docker_cert_path = Some(value.trim_matches('"').to_string());
        } else if let Some(value) = line.strip_prefix("DOCKER_TLS_VERIFY=") {
            docker_tls_verify = Some(value.trim_matches('"').to_string());
        }
    }

    // Build docker command with minikube's environment
    let mut cmd = Command::new("docker");
    cmd.arg("build")
        .arg("-t")
        .arg(tag)
        .arg("-f")
        .arg(dockerfile);

    // Set minikube docker environment variables
    if let Some(host) = docker_host {
        cmd.env("DOCKER_HOST", host);
    }
    if let Some(cert_path) = docker_cert_path {
        cmd.env("DOCKER_CERT_PATH", cert_path);
    }
    if let Some(tls_verify) = docker_tls_verify {
        cmd.env("DOCKER_TLS_VERIFY", tls_verify);
    }

    for (key, value) in build_args {
        cmd.arg("--build-arg").arg(format!("{}={}", key, value));
    }

    cmd.arg(workspace_root);

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
        Err(anyhow!("Failed to build Docker image {} for minikube", tag))
    }
}
