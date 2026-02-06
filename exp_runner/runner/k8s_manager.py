"""
Kubernetes operations manager for deploying and managing experiments on K8s.
"""

import json
import logging
import subprocess
import tempfile
import threading
import time
from pathlib import Path
from typing import Dict, List, Optional, Tuple

from .deployment_manager import DeploymentManager, TaskSpec

# Attempt to import strip_ansi_codes from docker_manager if available
try:
    from .docker_manager import strip_ansi_codes
except ImportError:
    import re

    def strip_ansi_codes(text: str) -> str:
        ansi_escape = re.compile(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])")
        return ansi_escape.sub("", text)


logger = logging.getLogger(__name__)


class K8sManager(DeploymentManager):
    """
    Manages Kubernetes operations for experiments using Helm and Kubectl.
    """

    def __init__(
        self,
        repo_root: Path,
        kube_context: Optional[str] = None,
        namespace: str = "default",
    ):
        self.repo_root = repo_root
        self.kube_context = kube_context
        self.namespace = namespace

    def _run_cmd(
        self, cmd: List[str], check: bool = True, capture_output: bool = True
    ) -> subprocess.CompletedProcess:
        """Helper to run shell commands with context handling."""
        final_cmd = cmd.copy()

        # Note: We handle context/namespace per-command usually, but could inject here if needed globally.
        # Most kubectl/helm commands accept --context.

        try:
            result = subprocess.run(
                final_cmd, check=check, capture_output=capture_output, text=True
            )
            return result
        except subprocess.CalledProcessError as e:
            if check:
                logger.error(f"Command failed: {' '.join(final_cmd)}")
                logger.error(f"Stdout: {e.stdout}")
                logger.error(f"Stderr: {e.stderr}")
                raise
            return e

    def _get_pod_phase(self, pod_name: str) -> str:
        cmd = [
            "kubectl",
            "get",
            "pod",
            pod_name,
            "-n",
            self.namespace,
            "-o",
            "jsonpath={.status.phase}",
        ]
        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        try:
            res = self._run_cmd(cmd, capture_output=True)
            return res.stdout.strip()
        except subprocess.CalledProcessError:
            return "Unknown"

    def run_task(self, task_spec: TaskSpec, log_file: Optional[Path] = None) -> None:
        """
        Run a one-off task using a K8s Pod.
        """
        logger.info(f"Running task {task_spec.name} on K8s")

        # Clean up existing pod
        del_cmd = [
            "kubectl",
            "delete",
            "pod",
            task_spec.name,
            "--namespace",
            self.namespace,
            "--ignore-not-found",
        ]
        if self.kube_context:
            del_cmd.extend(["--context", self.kube_context])
        self._run_cmd(del_cmd)

        # Handle volumes (create ConfigMaps)
        volume_mounts = []
        volumes = []
        created_configmaps = []

        for host_path, container_path in task_spec.volumes.items():
            path_obj = Path(host_path)
            if path_obj.is_file():
                # Sanitize name
                safe_name = path_obj.stem.lower().replace("_", "-")
                cm_name = f"{task_spec.name}-{safe_name}-cm"[:63]  # Limit length

                # Create CM
                create_cm_cmd = [
                    "kubectl",
                    "create",
                    "cm",
                    cm_name,
                    "--namespace",
                    self.namespace,
                    f"--from-file={path_obj.name}={host_path}",
                ]
                if self.kube_context:
                    create_cm_cmd.extend(["--context", self.kube_context])

                # Delete existing CM if any
                del_cm_cmd = [
                    "kubectl",
                    "delete",
                    "cm",
                    cm_name,
                    "--namespace",
                    self.namespace,
                    "--ignore-not-found",
                ]
                if self.kube_context:
                    del_cm_cmd.extend(["--context", self.kube_context])
                self._run_cmd(del_cm_cmd)

                self._run_cmd(create_cm_cmd)
                created_configmaps.append(cm_name)

                vol_name = f"vol-{len(volumes)}"
                volumes.append({"name": vol_name, "configMap": {"name": cm_name}})
                volume_mounts.append(
                    {
                        "name": vol_name,
                        "mountPath": container_path,
                        "subPath": path_obj.name,
                    }
                )
            else:
                logger.warning(
                    f"Skipping volume {host_path}: only single file mounts supported in K8sManager currently."
                )

        # Construct Pod Manifest
        pod_manifest = {
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {
                "name": task_spec.name,
                "namespace": self.namespace,
                "labels": {"app": task_spec.name},
            },
            "spec": {
                "restartPolicy": "Never",
                "containers": [
                    {
                        "name": task_spec.name,
                        "image": task_spec.image,
                        "imagePullPolicy": "IfNotPresent",
                        "env": [
                            {"name": k, "value": str(v)}
                            for k, v in task_spec.env_vars.items()
                        ],
                        "volumeMounts": volume_mounts,
                    }
                ],
                "volumes": volumes,
            },
        }

        if task_spec.command:
            pod_manifest["spec"]["containers"][0]["command"] = task_spec.command

        # Apply manifest
        with tempfile.NamedTemporaryFile(mode="w", suffix=".yaml") as f:
            json.dump(pod_manifest, f)
            f.flush()
            apply_cmd = ["kubectl", "apply", "-f", f.name]
            if self.kube_context:
                apply_cmd.extend(["--context", self.kube_context])
            self._run_cmd(apply_cmd)

        # Wait for completion
        logger.info(f"Waiting for task {task_spec.name} to complete...")

        try:
            # Poll for status
            found_pattern = False
            phase = "Unknown"

            while True:
                phase = self._get_pod_phase(task_spec.name)

                if task_spec.wait_for_log_pattern:
                    # Check logs for pattern
                    cmd_logs = ["kubectl", "logs", task_spec.name, "-n", self.namespace]
                    if self.kube_context:
                        cmd_logs.extend(["--context", self.kube_context])

                    try:
                        res = self._run_cmd(cmd_logs, check=False, capture_output=True)
                        if task_spec.wait_for_log_pattern in res.stdout:
                            found_pattern = True
                            break
                    except Exception:
                        pass

                if phase in ["Succeeded", "Failed"]:
                    break

                time.sleep(2)

            if log_file:
                self._stream_pod_log(task_spec.name, log_file, follow=False)

            if task_spec.wait_for_log_pattern:
                if not found_pattern:
                    # Check one last time in case it finished right after last check
                    cmd_logs = ["kubectl", "logs", task_spec.name, "-n", self.namespace]
                    if self.kube_context:
                        cmd_logs.extend(["--context", self.kube_context])
                    res = self._run_cmd(cmd_logs, check=False, capture_output=True)
                    if task_spec.wait_for_log_pattern in res.stdout:
                        found_pattern = True

                if not found_pattern:
                    raise RuntimeError(
                        f"Task {task_spec.name} finished with phase {phase} but pattern '{task_spec.wait_for_log_pattern}' not found"
                    )
            elif phase != "Succeeded":
                raise RuntimeError(f"Task {task_spec.name} failed with phase {phase}")

            # Copy artifacts if succeeded or pattern found
            should_copy = (phase == "Succeeded") or (
                task_spec.wait_for_log_pattern and found_pattern
            )

            if should_copy and task_spec.artifacts:
                for src, dest in task_spec.artifacts:
                    try:
                        self.copy_from_container(task_spec.name, src, Path(dest))
                    except Exception as e:
                        logger.warning(f"Failed to copy artifact {src} -> {dest}: {e}")

        finally:
            if task_spec.cleanup:
                self.cleanup_task(task_spec)

    def cleanup_task(self, task_spec: TaskSpec) -> None:
        """
        Cleanup task resources (Pod and ConfigMaps).
        """
        del_pod = [
            "kubectl",
            "delete",
            "pod",
            task_spec.name,
            "--namespace",
            self.namespace,
            "--ignore-not-found",
        ]
        if self.kube_context:
            del_pod.extend(["--context", self.kube_context])
        self._run_cmd(del_pod, check=False)

        # Cleanup ConfigMaps derived from volumes
        for host_path, _ in task_spec.volumes.items():
            path_obj = Path(host_path)
            if path_obj.is_file():
                # Reconstruct CM name
                safe_name = path_obj.stem.lower().replace("_", "-")
                cm_name = f"{task_spec.name}-{safe_name}-cm"[:63]

                del_cm = [
                    "kubectl",
                    "delete",
                    "cm",
                    cm_name,
                    "--namespace",
                    self.namespace,
                    "--ignore-not-found",
                ]
                if self.kube_context:
                    del_cm.extend(["--context", self.kube_context])
                self._run_cmd(del_cm, check=False)

    def start(
        self,
        app_dir: Path,
        deployment_config: str,
        env_vars: Dict[str, str],
        project_name: str,
    ) -> None:
        """
        Start services using Helm.
        """
        # In K8s mode, deployment_config is interpreted as the chart path
        chart_path = app_dir / deployment_config

        logger.info(
            f"Installing helm chart {project_name} from {chart_path} in namespace {self.namespace}..."
        )

        cmd = ["helm", "upgrade", "--install", project_name, str(chart_path)]
        cmd.extend(["--namespace", self.namespace, "--create-namespace"])

        if self.kube_context:
            cmd.extend(["--kube-context", self.kube_context])

        # Handle environment variables
        # We look for HELM_VALUES_FILE in env_vars to pass generic values
        if "HELM_VALUES_FILE" in env_vars:
            cmd.extend(["-f", env_vars["HELM_VALUES_FILE"]])

        # Wait for deployment
        cmd.extend(["--wait", "--timeout", "300s"])

        try:
            self._run_cmd(cmd)
            logger.info(f"Helm chart {project_name} installed.")
        except subprocess.CalledProcessError:
            logger.error("Helm install failed. Collecting debug info...")

            # List pods
            logger.error("--- Pods ---")
            cmd_pods = ["kubectl", "get", "pods", "-n", self.namespace]
            if self.kube_context:
                cmd_pods.extend(["--context", self.kube_context])
            subprocess.run(cmd_pods, check=False)

            # List deployments
            logger.error("--- Deployments ---")
            cmd_deploy = ["kubectl", "get", "deployments", "-n", self.namespace]
            if self.kube_context:
                cmd_deploy.extend(["--context", self.kube_context])
            subprocess.run(cmd_deploy, check=False)

            # Describe deployments
            logger.error("--- Describe Deployments ---")
            cmd_desc_deploy = [
                "kubectl",
                "describe",
                "deployments",
                "-n",
                self.namespace,
            ]
            if self.kube_context:
                cmd_desc_deploy.extend(["--context", self.kube_context])
            subprocess.run(cmd_desc_deploy, check=False)

            # Events
            logger.error("--- Events ---")
            cmd_events = [
                "kubectl",
                "get",
                "events",
                "-n",
                self.namespace,
                "--sort-by=.lastTimestamp",
            ]
            if self.kube_context:
                cmd_events.extend(["--context", self.kube_context])
            subprocess.run(cmd_events, check=False)

            # Describe pods
            logger.error("--- Describe Pods ---")
            cmd_desc = ["kubectl", "describe", "pods", "-n", self.namespace]
            if self.kube_context:
                cmd_desc.extend(["--context", self.kube_context])
            subprocess.run(cmd_desc, check=False)

            # Logs
            label_selector = f"app.kubernetes.io/instance={project_name}"
            try:
                pods = self.get_pod_names(label_selector)
                for pod in pods:
                    logger.error(f"--- Logs for {pod} ---")
                    cmd_logs = [
                        "kubectl",
                        "logs",
                        pod,
                        "-n",
                        self.namespace,
                        "--tail=50",
                    ]
                    if self.kube_context:
                        cmd_logs.extend(["--context", self.kube_context])
                    subprocess.run(cmd_logs, check=False)
            except Exception as e:
                logger.error(f"Failed to get pod logs: {e}")

            raise

    def stop(
        self,
        app_dir: Path,
        deployment_config: str,
        env_vars: Optional[Dict[str, str]] = None,
        project_name: Optional[str] = None,
    ) -> None:
        """
        Stop services (uninstall Helm release).
        """
        if not project_name:
            logger.warning("No project_name provided to stop, skipping k8s uninstall")
            return

        cmd = ["helm", "uninstall", project_name, "--namespace", self.namespace]
        if self.kube_context:
            cmd.extend(["--kube-context", self.kube_context])

        cmd.append("--wait")

        logger.info(f"Uninstalling helm release {project_name}...")
        self._run_cmd(cmd, check=False)  # Don't fail if already gone
        logger.info(f"Helm release {project_name} uninstalled.")

    def get_pod_names(self, label_selector: str) -> list[str]:
        """
        Get list of pod names matching a label selector.
        """
        cmd = [
            "kubectl",
            "get",
            "pods",
            "-n",
            self.namespace,
            "-l",
            label_selector,
            "-o",
            "jsonpath={.items[*].metadata.name}",
        ]

        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        res = self._run_cmd(cmd)
        return res.stdout.strip().split()

    def get_container_names(
        self,
        config_path: Path,
        project_name: str,
        env_vars: Optional[Dict[str, str]] = None,
    ) -> List[str]:
        """
        Get list of pod names for the release.
        """
        # Selector for the release
        label_selector = f"app.kubernetes.io/instance={project_name}"
        return self.get_pod_names(label_selector)

    def stream_logs(
        self,
        container_names: List[str],
        output_dir: Path,
        follow: bool = True,
    ) -> List[threading.Thread]:
        """
        Stream logs from pods to files.
        """
        output_dir.mkdir(parents=True, exist_ok=True)
        threads = []

        for pod_name in container_names:
            log_file = output_dir / f"{pod_name}.log"

            if follow:
                thread = threading.Thread(
                    target=self._stream_pod_log,
                    args=(pod_name, log_file, True),
                    daemon=True,
                )
                thread.start()
                threads.append(thread)
                logger.debug(f"Started log streaming for {pod_name}")
            else:
                self._stream_pod_log(pod_name, log_file, False)

        return threads

    def _stream_pod_log(self, pod_name: str, log_file: Path, follow: bool) -> None:
        cmd = ["kubectl", "logs", "-n", self.namespace, pod_name]
        if follow:
            cmd.append("-f")

        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        try:
            if follow:
                with open(log_file, "w", encoding="utf-8") as f:
                    process = subprocess.Popen(
                        cmd,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        text=True,
                        bufsize=1,
                    )
                    try:
                        # Read line by line until process exits
                        for line in iter(process.stdout.readline, ""):
                            if line:
                                f.write(strip_ansi_codes(line))
                                f.flush()
                    finally:
                        process.terminate()
            else:
                res = self._run_cmd(cmd, check=False)  # Pod might be gone
                with open(log_file, "w", encoding="utf-8") as f:
                    f.write(strip_ansi_codes(res.stdout))
        except Exception as e:
            logger.warning(f"Error streaming logs for {pod_name}: {e}")

    def check_project_health(
        self,
        config_path: Path,
        project_name: str,
        env_vars: Optional[Dict[str, str]] = None,
    ) -> List[Tuple[str, int]]:
        """
        Check if any pods in the project have failed.
        """
        # Get pods and their statuses
        label_selector = f"app.kubernetes.io/instance={project_name}"

        cmd = [
            "kubectl",
            "get",
            "pods",
            "-n",
            self.namespace,
            "-l",
            label_selector,
            "-o",
            "jsonpath={range .items[*]}{.metadata.name},{.status.phase},{.status.containerStatuses[0].state.terminated.exitCode}{'\\n'}{end}",
        ]

        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        try:
            res = self._run_cmd(cmd)
            failed_pods = []
            for line in res.stdout.strip().split("\n"):
                if not line:
                    continue
                parts = line.split(",")
                if len(parts) < 2:
                    continue

                name = parts[0]
                phase = parts[1]
                exit_code_str = parts[2] if len(parts) > 2 else ""

                exit_code = int(exit_code_str) if exit_code_str else 0

                if phase in ["Failed", "Unknown"] or (
                    phase == "Running" and exit_code != 0
                ):
                    failed_pods.append((name, exit_code))

            return failed_pods
        except Exception as e:
            logger.warning(f"Failed to check k8s health: {e}")
            return []

    def port_forward(
        self, service_name: str, local_port: int, remote_port: int
    ) -> Optional[subprocess.Popen]:
        """
        Start port forwarding in a background process.
        Returns the Popen object so it can be terminated later.
        """
        cmd = [
            "kubectl",
            "port-forward",
            "-n",
            self.namespace,
            f"svc/{service_name}",
            f"{local_port}:{remote_port}",
        ]

        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        logger.info(
            f"Starting port-forward: {local_port}:{remote_port} -> {service_name}"
        )
        process = subprocess.Popen(
            cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )

        # Wait a bit to check if it fails immediately
        time.sleep(1)
        if process.poll() is not None:
            out, err = process.communicate()
            raise RuntimeError(f"Port forward failed immediately: {err}")

        return process

    def load_image_to_cluster(self, cluster_name: str, image_names: List[str]) -> None:
        """
        Load docker images into kind cluster.
        """
        for img in image_names:
            logger.info(f"Loading image {img} into kind cluster {cluster_name}...")
            cmd = ["kind", "load", "docker-image", img, "--name", cluster_name]
            self._run_cmd(cmd)

    def copy_from_container(
        self, container_name: str, src_path: str, dest_path: Path
    ) -> None:
        """
        Copy file/directory from a pod to local path.
        """
        cmd = [
            "kubectl",
            "cp",
            f"{container_name}:{src_path}",
            str(dest_path),
            "-n",
            self.namespace,
        ]

        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        self._run_cmd(cmd)
