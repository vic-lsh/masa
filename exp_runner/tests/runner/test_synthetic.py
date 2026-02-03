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

from exp_runner.runner.apps.synthetic import (
    SyntheticApp,
    SyntheticBuilder,
    SyntheticLoadGenerator,
    _safe_project_name,
)
from exp_runner.runner.apps.utils import normalize_features_to_tag


class TestSyntheticLoadGenerator:
    """Tests for SyntheticLoadGenerator with feature-based tags."""

    def test_image_name_with_features(self):
        """Test that load generator creates correct image name with features."""
        loadgen = SyntheticLoadGenerator(features="policy-a,policy-b")
        assert loadgen.get_image_name() == "synthetic_client_bench:policy-a-policy-b"

    def test_image_name_without_features(self):
        """Test that load generator uses 'latest' without features."""
        loadgen = SyntheticLoadGenerator(features=None)
        assert loadgen.get_image_name() == "synthetic_client_bench:latest"

        loadgen2 = SyntheticLoadGenerator(features="")
        assert loadgen2.get_image_name() == "synthetic_client_bench:latest"

    def test_container_name(self):
        """Test default container name."""
        loadgen = SyntheticLoadGenerator(features="test")
        assert loadgen.get_container_name() == "synthetic_client_bench"

    def test_container_name_with_project(self):
        """Test that container name is namespaced with a project."""
        loadgen = SyntheticLoadGenerator(features="test", project_name="proj")
        assert loadgen.get_container_name() == "proj_synthetic_client_bench"

    def test_network_name(self):
        """Test default network name for static compose."""
        loadgen = SyntheticLoadGenerator(features="test")
        assert loadgen.get_network_name() == "local_synthetic_network"

    def test_network_name_with_project(self):
        """Test that compose project network is derived from project name."""
        loadgen = SyntheticLoadGenerator(features="test", project_name="proj")
        assert loadgen.get_network_name() == "proj_synthetic_network"

    def test_network_name_override(self):
        loadgen = SyntheticLoadGenerator(
            features="test", project_name="proj", network_name="net"
        )
        assert loadgen.get_network_name() == "net"

    def test_binary_name(self):
        """Test that binary name is constant."""
        loadgen = SyntheticLoadGenerator(features="test")
        assert loadgen.get_binary_name() == "synthetic_client_bench"

    def test_features_preserved(self):
        """Test that features are stored correctly."""
        features = "policy-x,policy-y"
        loadgen = SyntheticLoadGenerator(features=features)
        assert loadgen.features == features


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

    def test_create_load_generator_with_features(self):
        """Test that app creates load generator with features."""
        app = SyntheticApp()
        features = "policy-x,policy-y"

        loadgen = app.create_load_generator(features=features)

        assert isinstance(loadgen, SyntheticLoadGenerator)
        assert loadgen.features == features
        assert loadgen.get_image_name() == "synthetic_client_bench:policy-x-policy-y"

    def test_safe_project_name(self):
        name = _safe_project_name(
            experiment_name="exp 1", iteration=0, policy="fifo,early"
        )
        # docker compose project name allowed chars: [a-z0-9_-]
        assert "," not in name
        assert " " not in name
        assert name.startswith("syn-")
        assert all(c.islower() or c.isdigit() or c in "-_" for c in name)

    def test_create_load_generator_without_features(self):
        """Test that app creates load generator without features."""
        app = SyntheticApp()

        loadgen = app.create_load_generator(features=None)

        assert isinstance(loadgen, SyntheticLoadGenerator)
        assert loadgen.features is None
        assert loadgen.get_image_name() == "synthetic_client_bench:latest"

    def test_create_builder(self):
        """Test that app creates correct builder."""
        app = SyntheticApp()
        builder = app.create_builder()

        assert isinstance(builder, SyntheticBuilder)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
