"""
Tests for SyntheticWorkloadRunner and its subclasses.
"""

from pathlib import Path
from unittest.mock import MagicMock, Mock, patch

import pytest

from exp_runner.runner.apps.synthetic import (
    DockerSyntheticRunner,
    K8sSyntheticRunner,
    SyntheticApp,
)
from exp_runner.runner.docker_manager import DockerManager
from exp_runner.runner.k8s_manager import K8sManager


class TestSyntheticWorkloadRunner:
    @patch("exp_runner.runner.apps.synthetic.SyntheticApp.get_docker_config")
    def test_run_workload_dispatch_docker(self, mock_get_docker_config, tmp_path):
        """Test that run_workload dispatches to DockerSyntheticRunner for DockerManager."""
        app = SyntheticApp()
        mock_get_docker_config.return_value = Mock(
            app_config_filename=None, compose_file="docker-compose.yaml"
        )

        # Mock DockerManager
        docker_manager = Mock(spec=DockerManager)
        docker_manager.get_container_names.return_value = ["c1"]

        # Mock load generator
        app.create_load_generator = Mock()

        # Setup mocks
        config = Mock()
        config.gen_config = {"Addr": "http://localhost:8000"}
        config.app_config = {}
        config.app_dir = Path("/tmp")
        config.in_dir = tmp_path
        config.experiment_name = "test-exp"

        (tmp_path / "gen_config.json").write_text("{}")

        output_dir = tmp_path / "out"

        app.run_workload(
            repo_root=Path("/repo"),
            config=config,
            docker=docker_manager,
            policy="fifo",
            iteration=0,
            output_dir=output_dir,
            app_local_dir=Path("/local"),
            no_cache=False,
        )

        # Verification: Docker runner does NOT create values.yaml
        assert not (output_dir / "values.yaml").exists()
        # It should create .env
        assert (output_dir / ".env").exists()

    @patch("exp_runner.runner.apps.synthetic.SyntheticApp.get_docker_config")
    def test_run_workload_dispatch_k8s(self, mock_get_docker_config, tmp_path):
        """Test that run_workload dispatches to K8sSyntheticRunner for K8sManager."""
        app = SyntheticApp()
        mock_get_docker_config.return_value = Mock(
            app_config_filename=None, compose_file="docker-compose.yaml"
        )

        # Mock K8sManager
        k8s_manager = Mock(spec=K8sManager)
        k8s_manager.get_container_names.return_value = ["p1"]
        # Ensure fallback works if isinstance fails (though it shouldn't)
        k8s_manager.kube_context = "test-ctx"

        app.create_load_generator = Mock()

        config = Mock()
        config.gen_config = {"Addr": "http://localhost:8000"}
        config.app_config = {}
        config.app_dir = Path("/tmp")
        config.in_dir = tmp_path
        config.experiment_name = "test-exp"

        (tmp_path / "gen_config.json").write_text("{}")

        output_dir = tmp_path / "out"

        app.run_workload(
            repo_root=Path("/repo"),
            config=config,
            docker=k8s_manager,
            policy="fifo",
            iteration=0,
            output_dir=output_dir,
            app_local_dir=Path("/local"),
            no_cache=False,
        )

        # Verification: K8s runner creates values.yaml
        assert (output_dir / "values.yaml").exists()
        # It also creates .env
        assert (output_dir / ".env").exists()

    def test_docker_runner_service_name(self):
        """Test Docker runner returns None for service override."""
        app = Mock(spec=SyntheticApp)
        manager = Mock(spec=DockerManager)
        runner = DockerSyntheticRunner(app, manager)
        assert runner.get_service_name_override("proj") is None

    def test_k8s_runner_service_name(self):
        """Test K8s runner returns correct service override."""
        app = Mock(spec=SyntheticApp)
        manager = Mock(spec=K8sManager)
        runner = K8sSyntheticRunner(app, manager)
        assert runner.get_service_name_override("proj") == "proj-frontend"
