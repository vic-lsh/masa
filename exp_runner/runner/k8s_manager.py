"""
Kubernetes operations manager for deploying and managing experiments on K8s.
"""

import logging
import subprocess
import threading
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

# Attempt to import strip_ansi_codes from docker_manager if available
try:
    from .docker_manager import strip_ansi_codes
except ImportError:
    import re

    def strip_ansi_codes(text: str) -> str:
        ansi_escape = re.compile(r"\x1B(?:[@-Z\\-_]|\[[0-?]*[ -/]*[@-~])")
        return ansi_escape.sub("", text)


logger = logging.getLogger(__name__)


class K8sManager:
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

    def start(
        self,
        app_dir: Path,
        compose_file: str,  # Treated as chart path relative to app_dir or absolute
        env_vars: dict,
        project_name: str | None = None,
    ) -> None:
        """
        Start services using Helm.
        Matches DockerManager.start interface.
        """
        if not project_name:
            raise ValueError("project_name is required for K8s deployment")

        # In K8s mode, compose_file is interpreted as the chart path
        chart_path = app_dir / compose_file
        if not chart_path.exists():
            # Fallback: maybe it's just the name of the chart directory in app_dir
            pass

        logger.info(
            f"Installing helm chart {project_name} from {chart_path} in namespace {self.namespace}..."
        )

        cmd = ["helm", "upgrade", "--install", project_name, str(chart_path)]
        cmd.extend(["--namespace", self.namespace, "--create-namespace"])

        if self.kube_context:
            cmd.extend(["--kube-context", self.kube_context])

        # Handle environment variables by creating a temporary values file
        if env_vars:
            import json
            import tempfile

            import yaml

            values = {}

            # Map APP_CONFIG_PATH to appConfig
            if "APP_CONFIG_PATH" in env_vars:
                try:
                    with open(env_vars["APP_CONFIG_PATH"]) as f:
                        app_config = json.load(f)
                    values["appConfig"] = app_config
                except Exception as e:
                    logger.warning(
                        f"Failed to load app config from {env_vars['APP_CONFIG_PATH']}: {e}"
                    )

            # Map LOG_LEVEL
            if "LOG_LEVEL" in env_vars:
                values["logLevel"] = env_vars["LOG_LEVEL"]

            # Map *_IMAGE_TAG to image.tag
            for k, v in env_vars.items():
                if k.endswith("_IMAGE_TAG"):
                    if "image" not in values:
                        values["image"] = {}
                    values["image"]["tag"] = v
                    break

            # Use a temporary file for values
            if values:
                with tempfile.NamedTemporaryFile(
                    mode="w", suffix=".yaml", delete=False
                ) as tmp:
                    yaml.dump(values, tmp)
                    values_file = tmp.name
                cmd.extend(["-f", values_file])

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
        compose_file: str,
        env_vars: dict | None = None,
        project_name: str | None = None,
    ) -> None:
        """
        Stop services (uninstall Helm release).
        Matches DockerManager.stop interface.
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
        compose_path: Path,
        project_name: str,
        env_vars: dict | None = None,
    ) -> list[str]:
        """
        Get list of pod names for the release.
        Matches DockerManager.get_container_names interface.
        """
        # Selector for the release
        label_selector = f"app.kubernetes.io/instance={project_name}"
        return self.get_pod_names(label_selector)

    def stream_logs(
        self,
        container_names: list[str],
        output_dir: Path,
        follow: bool = True,
    ) -> list[threading.Thread]:
        """
        Stream logs from pods to files.
        Matches DockerManager.stream_logs interface.
        """
        # Rename arg for internal consistency, but interface uses container_names
        pod_names = container_names

        output_dir.mkdir(parents=True, exist_ok=True)
        threads = []

        for pod_name in pod_names:
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
        compose_path: Path,
        project_name: str,
        env_vars: dict | None = None,
    ) -> list[tuple[str, int]]:
        """
        Check if any pods in the project have failed.
        Matches DockerManager.check_project_health interface.
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
    ) -> subprocess.Popen:
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

    def load_image_to_kind(self, cluster_name: str, image_names: List[str]) -> None:
        """
        Load docker images into kind cluster.
        """
        for img in image_names:
            logger.info(f"Loading image {img} into kind cluster {cluster_name}...")
            cmd = ["kind", "load", "docker-image", img, "--name", cluster_name]
            self._run_cmd(cmd)

    def copy_from_pod(self, pod_name: str, src_path: str, dest_path: Path) -> None:
        """
        Copy file/directory from a pod to local path.
        """
        cmd = [
            "kubectl",
            "cp",
            f"{pod_name}:{src_path}",
            str(dest_path),
            "-n",
            self.namespace,
        ]

        if self.kube_context:
            cmd.extend(["--context", self.kube_context])

        self._run_cmd(cmd)
