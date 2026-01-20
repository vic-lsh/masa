"""
Kubernetes operations manager using Helm and Kubectl.
"""

import logging
import os
import subprocess
import threading
import time
import json
from pathlib import Path
from typing import Optional

from .deployment_manager import DeploymentManager

logger = logging.getLogger(__name__)


class K8sManager(DeploymentManager):
    """
    Manages Kubernetes deployments using Helm.
    """

    def __init__(self, repo_root: Path):
        super().__init__(repo_root)
        self.namespace = "default"  # Could be configurable

    def start(
        self,
        app_dir: Path,
        config: dict,
        env_vars: dict,
        project_name: Optional[str] = None,
    ) -> None:
        """
        Deploy application using Helm.

        Args:
            app_dir: Path to application directory
            config: Dict containing 'chart_path' and 'values'
            env_vars: Environment variables to pass to Helm (merged into values)
            project_name: Release name for Helm
        """
        if not project_name:
            raise ValueError("project_name is required for K8s deployment")

        chart_path = config.get("chart_path")
        if not chart_path:
            raise ValueError("K8s config must contain 'chart_path'")

        # Resolve chart path relative to app_dir
        full_chart_path = app_dir / chart_path

        logger.info(f"Deploying Helm release {project_name} from {full_chart_path}")

        # Construct Helm command
        cmd = [
            "helm",
            "upgrade",
            "--install",
            project_name,
            str(full_chart_path),
            "--wait",  # Wait for pods to be ready
            "--timeout",
            "5m",
        ]

        # Add values from config
        if "values" in config and config["values"]:
            # If values is a dict, we might need to write it to a temp file or use --set
            # For now, let's assume it's a path to a values file if it's a string,
            # or we handle dicts by converting to --set or -f
            pass

        # Pass environment variables as Helm values
        # We assume the chart has an 'env' section or similar
        # For the synthetic chart, we used 'env.LOG_LEVEL', etc.
        # But we also have specific values like 'frontend.port'.
        # We need a way to map flat env vars to nested Helm values?
        # Or we just pass the flat env vars if the chart supports it.
        # The synthetic chart I wrote supports 'env' dict.

        # Let's write a temporary values file for the env vars
        # This is cleaner than many --set arguments
        values = {"env": env_vars}

        # Allow overriding other values if provided in config
        if "helm_values" in config:
            values.update(config["helm_values"])

        # We can write this to a temp file
        import tempfile

        with tempfile.NamedTemporaryFile(mode="w", suffix=".yaml", delete=False) as tmp:
            import yaml

            yaml.dump(values, tmp)
            tmp_values_path = tmp.name

        cmd.extend(["-f", tmp_values_path])

        try:
            subprocess.run(cmd, check=True, capture_output=True, text=True)
            logger.info(f"Helm release {project_name} deployed successfully")
        except subprocess.CalledProcessError as e:
            logger.error(f"Helm deploy failed: {e.stderr}")
            raise
        finally:
            os.unlink(tmp_values_path)

    def stop(
        self,
        app_dir: Path,
        config: dict,
        env_vars: Optional[dict] = None,
        project_name: Optional[str] = None,
    ) -> None:
        """
        Uninstall Helm release.
        """
        if not project_name:
            return

        logger.info(f"Uninstalling Helm release {project_name}")

        cmd = ["helm", "uninstall", project_name, "--wait"]

        try:
            subprocess.run(
                cmd,
                check=False,  # Don't fail if already gone
                capture_output=True,
                text=True,
            )
            logger.info(f"Helm release {project_name} uninstalled")
        except subprocess.CalledProcessError as e:
            logger.warning(f"Helm uninstall failed: {e}")

    def get_container_names(
        self,
        config: dict,
        project_name: str,
        env_vars: Optional[dict] = None,
    ) -> list[str]:
        """
        Get list of pod names for the release.
        """
        # kubectl get pods -l app.kubernetes.io/instance={project_name} -o jsonpath='{.items[*].metadata.name}'
        # Note: Helm adds 'app.kubernetes.io/instance' label by default?
        # Actually my chart uses 'app: {{ .Release.Name }}-frontend' etc.
        # I should use a label selector that matches the release.
        # Standard helm charts use 'app.kubernetes.io/instance'.
        # My chart didn't explicitly add standard labels, but I can match by name prefix or add labels.
        # Let's check my chart. I used `app: {{ .Release.Name }}-frontend`.
        # So I can just list pods and filter? Or use a selector.

        # Better: assume standard labels or implementation specific.
        # For my synthetic chart, I set `app` label.
        # But `app` label varies (frontend vs child).
        # But they all contain the release name in the name.

        cmd = [
            "kubectl",
            "get",
            "pods",
            "--no-headers",
            "-o",
            "custom-columns=:metadata.name",
        ]

        try:
            result = subprocess.run(cmd, check=True, capture_output=True, text=True)
            all_pods = result.stdout.splitlines()
            # Filter for pods starting with release name
            # My chart names deployments: {{ .Release.Name }}-frontend
            # Pods will be {{ .Release.Name }}-frontend-xyz
            return [p for p in all_pods if p.startswith(project_name)]
        except subprocess.CalledProcessError as e:
            logger.warning(f"Failed to get pod names: {e}")
            return []

    def stream_logs(
        self, container_names: list[str], output_dir: Path, follow: bool = True
    ) -> list[threading.Thread]:
        """
        Stream logs from pods.
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
            else:
                self._stream_pod_log(pod_name, log_file, False)

        return threads

    def _stream_pod_log(self, pod_name: str, log_file: Path, follow: bool) -> None:
        cmd = ["kubectl", "logs"]
        if follow:
            cmd.append("-f")
        cmd.append(pod_name)

        # Note: If pod has multiple containers, we might need -c.
        # My chart has 1 container per pod for now.

        try:
            with open(log_file, "w", encoding="utf-8") as f:
                if follow:
                    process = subprocess.Popen(
                        cmd,
                        stdout=subprocess.PIPE,
                        stderr=subprocess.STDOUT,
                        text=True,
                        bufsize=1,
                    )
                    if process.stdout:
                        for line in iter(process.stdout.readline, ""):
                            if line:
                                f.write(line)
                                f.flush()
                    process.wait()
                else:
                    subprocess.run(cmd, stdout=f, stderr=subprocess.STDOUT, check=False)
        except Exception as e:
            logger.warning(f"Error streaming logs from {pod_name}: {e}")
