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

pub fn build_images(workspace_root: &Path, trace_dir: &Path, service_config_dir: &Path) -> Result<Images> {
    info!("Building Docker images for Kubernetes backend.");
    info!("Trace data will be baked into the images from {:?}", trace_dir);

    // Canonicalize paths first to handle relative paths like ./service_configs
    let trace_dir_abs = std::fs::canonicalize(trace_dir)
        .with_context(|| format!("Failed to canonicalize trace directory {:?}", trace_dir))?;
    let service_config_dir_abs = std::fs::canonicalize(service_config_dir)
        .with_context(|| format!("Failed to canonicalize service config directory {:?}", service_config_dir))?;
    let workspace_root_abs = std::fs::canonicalize(workspace_root)
        .with_context(|| format!("Failed to canonicalize workspace root {:?}", workspace_root))?;

    // Convert directories to paths relative to workspace root for Docker build context
    let trace_dir_rel = trace_dir_abs.strip_prefix(&workspace_root_abs)
        .with_context(|| format!("Trace directory {:?} is not inside workspace {:?}", trace_dir_abs, workspace_root_abs))?
        .to_string_lossy()
        .to_string();

    let service_config_dir_rel = service_config_dir_abs.strip_prefix(&workspace_root_abs)
        .with_context(|| format!("Service config directory {:?} is not inside workspace {:?}", service_config_dir_abs, workspace_root_abs))?
        .to_string_lossy()
        .to_string();

    info!("Trace directory (relative to workspace): {}", trace_dir_rel);
    info!("Config directory (relative to workspace): {}", service_config_dir_rel);

    let generic_dockerfile = workspace_root.join("apps/mssim/generic-service/Dockerfile.k8s");

    // Check if k8s-specific Dockerfile exists, otherwise use the regular one
    let dockerfile_to_use = if generic_dockerfile.exists() {
        info!("Using Kubernetes-specific Dockerfile that bakes in trace data");
        generic_dockerfile
    } else {
        info!("Using standard Dockerfile (trace data won't be baked in)");
        workspace_root.join("apps/mssim/generic-service/Dockerfile")
    };

    build_docker_image(
        GENERIC_SERVICE_IMAGE_TAG,
        &dockerfile_to_use,
        &[
            ("SERVICE_CONTAINER_PORT", "50051".to_string()),
            ("TRACE_DATA_PATH", trace_dir_rel),
            ("CONFIG_DATA_PATH", service_config_dir_rel),
        ],
        workspace_root,
    )?;

    let loadgen_dockerfile = workspace_root.join("apps/mssim/generic-service/Dockerfile.loadgen");
    build_docker_image(
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
    info!("Note: Trace data and configs are baked into the Docker images");

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
                            "imagePullPolicy": "IfNotPresent",
                            "env": [
                                {"name": "SERVICE_NAME", "value": svc_name.clone()},
                                {"name": "SERVICE_PORT", "value": svc_port.to_string()},
                                {"name": "CONFIG_PATH", "value": "/app/config"},
                                {"name": "DEPLOYMEN_CONFIG_PATH", "value": "/app/config/deployment.json"}
                            ],
                            "ports": [{"containerPort": svc_port}],
                            "resources": {
                                "limits": {"cpu": format!("{}m", container_cpu_limit * 100)}
                            }
                        }]
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
                        "imagePullPolicy": "IfNotPresent",
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

fn build_docker_image(
    tag: &str,
    dockerfile: &Path,
    build_args: &[(&str, String)],
    workspace_root: &Path,
) -> Result<()> {
    info!("Building Docker image {} using {:?}", tag, dockerfile);
    info!("Build context: {:?}", workspace_root);

    // Build docker command
    let mut cmd = Command::new("docker");
    cmd.arg("build")
        .arg("-t")
        .arg(tag)
        .arg("-f")
        .arg(dockerfile);

    for (key, value) in build_args {
        let arg = format!("{}={}", key, value);
        info!("  --build-arg {}", arg);
        cmd.arg("--build-arg").arg(arg);
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
        info!("Successfully built image: {}", tag);
        info!("Note: For non-local clusters, push this image to a registry accessible by your cluster");
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

