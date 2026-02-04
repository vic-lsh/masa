"""
Tests for common building logic across applications.

This module tests shared build behavior including:
- Build cache ID consistency across all stages
- Feature-based cache ID generation
- Cache ID consistency across different apps (hotel, synthetic)
"""

import tempfile
from pathlib import Path
from unittest.mock import Mock, patch

import pytest

from exp_runner.runner.apps.hotel import HotelBuilder
from exp_runner.runner.apps.synthetic import SyntheticBuilder
from exp_runner.runner.apps.utils import normalize_features_to_tag

# Test fixtures for different builders
BUILDERS = [
    pytest.param(
        HotelBuilder,
        {
            "app_name": "hotel",
            "requires_app_config": True,
            "requires_gen_config": True,
        },
        id="hotel",
    ),
    pytest.param(
        SyntheticBuilder,
        {
            "app_name": "synthetic",
            "requires_app_config": False,
            "requires_gen_config": True,
        },
        id="synthetic",
    ),
]


class TestBuildCacheIDConsistency:
    """Tests for build cache ID consistency across all build stages."""

    @pytest.mark.parametrize("builder_class,config", BUILDERS)
    def test_cache_id_consistency_across_stages(self, builder_class, config):
        """Test that CACHE_ID is consistent across all stages and includes features."""
        # Patch the appropriate module based on builder type
        module_name = f"exp_runner.runner.apps.{config['app_name']}"

        with (
            patch(f"{module_name}.subprocess.run") as mock_subprocess,
            patch(f"{module_name}.logger"),
        ):
            builder = builder_class()
            mock_subprocess.return_value = Mock(returncode=0)

            with tempfile.TemporaryDirectory() as tmpdir:
                repo_root = Path(tmpdir) / "repo"
                app_dir = Path(tmpdir) / "app"
                repo_root.mkdir()
                app_dir.mkdir()

                # Set up config files based on requirements
                app_config_path = None
                gen_config_path = repo_root / "gen_config.json"

                if config["requires_app_config"]:
                    app_config_path = repo_root / f"{config['app_name']}.json"
                    app_config_path.write_text("{}")

                gen_config_path.write_text("{}")

                features = "policy-a,policy-b"

                # Build arguments
                build_kwargs = {
                    "repo_root": repo_root,
                    "app_dir": app_dir,
                    "features": features,
                    "rust_log": "info",
                    "no_cache": False,
                    "gen_config_path": gen_config_path,
                }

                builder.build(**build_kwargs)

                # Get expected cache ID
                tag = normalize_features_to_tag(features)
                expected_cache_id = f"{config['app_name']}-{tag}"

                # Extract all build commands
                all_calls = [call[0][0] for call in mock_subprocess.call_args_list]

                # Stage 1: Builder stage
                builder_call = all_calls[0]
                builder_call_str = " ".join(builder_call)
                assert f"CACHE_ID={expected_cache_id}" in builder_call_str, (
                    f"Builder stage should have CACHE_ID={expected_cache_id}"
                )

                # Stage 2: Runtime-base stage
                runtime_base_call = all_calls[1]
                runtime_base_call_str = " ".join(runtime_base_call)
                assert f"CACHE_ID={expected_cache_id}" in runtime_base_call_str, (
                    f"Runtime-base stage should have CACHE_ID={expected_cache_id}"
                )

                # Stage 3: Runtime stages (all binary images)
                runtime_calls = all_calls[2:]
                for runtime_call in runtime_calls:
                    runtime_call_str = " ".join(runtime_call)
                    assert f"CACHE_ID={expected_cache_id}" in runtime_call_str, (
                        f"Runtime stage should have CACHE_ID={expected_cache_id}"
                    )

                # Verify cache ID includes features (via tag)
                assert tag in expected_cache_id, (
                    f"Cache ID {expected_cache_id} should include tag {tag} which represents features"
                )

    @pytest.mark.parametrize("builder_class,config", BUILDERS)
    def test_cache_id_without_features(self, builder_class, config):
        """Test that CACHE_ID uses 'latest' tag when no features are provided."""
        # Patch the appropriate module based on builder type
        module_name = f"exp_runner.runner.apps.{config['app_name']}"

        with (
            patch(f"{module_name}.subprocess.run") as mock_subprocess,
            patch(f"{module_name}.logger"),
        ):
            builder = builder_class()
            mock_subprocess.return_value = Mock(returncode=0)

            with tempfile.TemporaryDirectory() as tmpdir:
                repo_root = Path(tmpdir) / "repo"
                app_dir = Path(tmpdir) / "app"
                repo_root.mkdir()
                app_dir.mkdir()

                # Set up config files based on requirements
                app_config_path = None
                gen_config_path = repo_root / "gen_config.json"

                if config["requires_app_config"]:
                    app_config_path = repo_root / f"{config['app_name']}.json"
                    app_config_path.write_text("{}")

                gen_config_path.write_text("{}")

                # Build arguments
                build_kwargs = {
                    "repo_root": repo_root,
                    "app_dir": app_dir,
                    "features": None,
                    "rust_log": "info",
                    "no_cache": False,
                    "gen_config_path": gen_config_path,
                }

                builder.build(**build_kwargs)

                # Expected cache ID without features
                expected_cache_id = f"{config['app_name']}-latest"

                # Extract all build commands
                all_calls = [call[0][0] for call in mock_subprocess.call_args_list]

                # Verify all stages have the same cache ID
                for call in all_calls:
                    call_str = " ".join(call)
                    assert f"CACHE_ID={expected_cache_id}" in call_str, (
                        f"All stages should have CACHE_ID={expected_cache_id}"
                    )

    @pytest.mark.parametrize("builder_class,config", BUILDERS)
    def test_cache_id_different_features_produce_different_ids(
        self, builder_class, config
    ):
        """Test that different features produce different cache IDs."""
        # Patch the appropriate module based on builder type
        module_name = f"exp_runner.runner.apps.{config['app_name']}"

        with (
            patch(f"{module_name}.subprocess.run") as mock_subprocess,
            patch(f"{module_name}.logger"),
        ):
            builder = builder_class()
            mock_subprocess.return_value = Mock(returncode=0)

            with tempfile.TemporaryDirectory() as tmpdir:
                repo_root = Path(tmpdir) / "repo"
                app_dir = Path(tmpdir) / "app"
                repo_root.mkdir()
                app_dir.mkdir()

                # Set up config files based on requirements
                app_config_path = None
                gen_config_path = repo_root / "gen_config.json"

                if config["requires_app_config"]:
                    app_config_path = repo_root / f"{config['app_name']}.json"
                    app_config_path.write_text("{}")

                gen_config_path.write_text("{}")

                # Build arguments helper
                def get_build_kwargs(features_val):
                    kwargs = {
                        "repo_root": repo_root,
                        "app_dir": app_dir,
                        "features": features_val,
                        "rust_log": "info",
                        "no_cache": False,
                        "gen_config_path": gen_config_path,
                    }
                    return kwargs

                # Build with first set of features
                features1 = "policy-a,policy-b"
                mock_subprocess.reset_mock()
                builder.build(**get_build_kwargs(features1))

                tag1 = normalize_features_to_tag(features1)
                cache_id1 = f"{config['app_name']}-{tag1}"

                # Build with different features
                features2 = "policy-c,policy-d"
                mock_subprocess.reset_mock()
                builder.build(**get_build_kwargs(features2))

                tag2 = normalize_features_to_tag(features2)
                cache_id2 = f"{config['app_name']}-{tag2}"

                # Verify cache IDs are different
                assert cache_id1 != cache_id2, (
                    f"Different features should produce different cache IDs: {cache_id1} vs {cache_id2}"
                )

                # Verify the second build uses the correct cache ID
                all_calls = [call[0][0] for call in mock_subprocess.call_args_list]
                builder_call = all_calls[0]
                builder_call_str = " ".join(builder_call)
                assert f"CACHE_ID={cache_id2}" in builder_call_str, (
                    f"Second build should use CACHE_ID={cache_id2}"
                )

    @pytest.mark.parametrize("builder_class,config", BUILDERS)
    def test_cache_id_format(self, builder_class, config):
        """Test that cache ID follows the expected format: {app}-{tag}."""
        # Patch the appropriate module based on builder type
        module_name = f"exp_runner.runner.apps.{config['app_name']}"

        with (
            patch(f"{module_name}.subprocess.run") as mock_subprocess,
            patch(f"{module_name}.logger"),
        ):
            builder = builder_class()
            mock_subprocess.return_value = Mock(returncode=0)

            with tempfile.TemporaryDirectory() as tmpdir:
                repo_root = Path(tmpdir) / "repo"
                app_dir = Path(tmpdir) / "app"
                repo_root.mkdir()
                app_dir.mkdir()

                # Set up config files based on requirements
                app_config_path = None
                gen_config_path = repo_root / "gen_config.json"

                if config["requires_app_config"]:
                    app_config_path = repo_root / f"{config['app_name']}.json"
                    app_config_path.write_text("{}")

                gen_config_path.write_text("{}")

                features = "test-feature"

                # Build arguments
                build_kwargs = {
                    "repo_root": repo_root,
                    "app_dir": app_dir,
                    "features": features,
                    "rust_log": "info",
                    "no_cache": False,
                    "gen_config_path": gen_config_path,
                }

                builder.build(**build_kwargs)

                # Extract cache ID from builder stage
                all_calls = [call[0][0] for call in mock_subprocess.call_args_list]
                builder_call = all_calls[0]

                # Find CACHE_ID in the command
                cache_id_parts = [
                    arg for arg in builder_call if arg.startswith("CACHE_ID=")
                ]
                assert len(cache_id_parts) > 0, (
                    "CACHE_ID should be present in build args"
                )

                cache_id = cache_id_parts[0].split("=", 1)[1]

                # Verify format: {app}-{tag}
                expected_prefix = f"{config['app_name']}-"
                assert cache_id.startswith(expected_prefix), (
                    f"Cache ID {cache_id} should start with {expected_prefix}"
                )

                # Verify tag part matches normalized features
                tag_part = cache_id[len(expected_prefix) :]
                expected_tag = normalize_features_to_tag(features)
                assert tag_part == expected_tag, (
                    f"Cache ID tag part {tag_part} should match normalized tag {expected_tag}"
                )


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
