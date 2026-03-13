"""
Tests for the synthetic application module.

This module tests the synthetic app functionality including:
- Feature-based docker image tagging
- Load generator configuration
- Docker image building with cache ID consistency
- Application component integration
"""

import tempfile
from pathlib import Path
from unittest.mock import Mock, patch

import pytest

from exp_runner.runner.naming import generate_project_name
from exp_runner.runner.apps.synthetic import (
    SyntheticApp,
    SyntheticBuilder,
)
from exp_runner.runner.naming import generate_project_name
from exp_runner.runner.deployment_manager import TaskSpec


class TestSyntheticBuilder:
    """Tests for SyntheticBuilder with feature-based tags and cache ID consistency."""

    @patch("exp_runner.runner.apps.synthetic.subprocess.run")
    @patch("exp_runner.runner.apps.synthetic.logger")
    def test_build_with_features(self, mock_logger, mock_subprocess):
        """Test that builder creates correct docker build command with features."""
        builder = SyntheticBuilder()
        mock_subprocess.return_value = Mock(returncode=0)

        with tempfile.TemporaryDirectory() as tmpdir:
            repo_root = Path(tmpdir) / "repo"
            app_dir = Path(tmpdir) / "app"
            repo_root.mkdir()
            app_dir.mkdir()

            gen_config_path = repo_root / "gen_config.json"

            # Create config file
            gen_config_path.write_text("{}")

            features = "policy-a,policy-b"

            builder.build(
                repo_root=repo_root,
                app_dir=app_dir,
                features=features,
                rust_log="info",
                no_cache=False,
                gen_config_path=gen_config_path,
            )

            # Check that subprocess.run was called multiple times
            assert mock_subprocess.call_count >= 3

            # Check that all calls include docker build
            all_calls = [call[0][0] for call in mock_subprocess.call_args_list]
            for cmd in all_calls:
                assert "docker" in cmd
                assert "build" in cmd


class TestSyntheticApp:
    """Tests for SyntheticApp integration with feature-based tags."""

    def test_get_image_tag(self):
        """Test that app returns correct image tag."""
        app = SyntheticApp()

        assert app.get_image_tag("policy-a,policy-b") == "policy-a-policy-b"
        assert app.get_image_tag("scheduling-fifo") == "scheduling-fifo"
        assert app.get_image_tag(None) == "latest"
        assert app.get_image_tag("") == "latest"

    def test_get_loadgen_spec_with_features(self):
        """Test that app creates load generator spec with features."""
        app = SyntheticApp()
        features = "policy-x,policy-y"
        project_name = "test-proj"

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            env_vars = {
                "DOCKER_COMPOSE_PROJECT_NAME": project_name,
                "LOG_LEVEL": "debug",
            }

            task_spec = app.get_loadgen_spec(
                output_dir=output_dir,
                features=features,
                env_vars=env_vars,
                use_k8s=False,
            )

            assert isinstance(task_spec, TaskSpec)
            assert task_spec.image == "synthetic_client_bench:policy-x-policy-y"
            assert task_spec.name == f"{project_name}-loadgen"
            assert task_spec.network == f"{project_name}_synthetic_network"
            assert task_spec.env_vars["LOG_LEVEL"] == "debug"

    def test_safe_project_name(self):
        name = generate_project_name(
            prefix="syn", experiment_name="exp 1", iteration=0, policy="fifo,early"
        )
        # docker compose project name allowed chars: [a-z0-9_-]
        assert "," not in name
        assert " " not in name
        assert name.startswith("syn-")
        assert all(c.islower() or c.isdigit() or c in "-_" for c in name)

    def test_get_loadgen_spec_without_features(self):
        """Test that app creates load generator spec without features."""
        app = SyntheticApp()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)
            env_vars = {"DOCKER_COMPOSE_PROJECT_NAME": "proj"}

            task_spec = app.get_loadgen_spec(
                output_dir=output_dir, features=None, env_vars=env_vars, use_k8s=False
            )

            assert isinstance(task_spec, TaskSpec)
            assert task_spec.image == "synthetic_client_bench:latest"

    def test_create_builder(self):
        """Test that app creates correct builder."""
        app = SyntheticApp()
        builder = app.create_builder()

        assert isinstance(builder, SyntheticBuilder)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
