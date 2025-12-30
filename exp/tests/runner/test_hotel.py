"""
Tests for the hotel application module.

This module tests the hotel app functionality including:
- Feature-based docker image tagging
- Load generator configuration
- Docker image building
- Application component integration
"""

import pytest
import tempfile
from pathlib import Path
from unittest.mock import Mock, patch

from exp.runner.apps.hotel import (
    HotelApp,
    HotelBuilder,
    HotelLoadGenerator,
)
from exp.runner.apps.utils import normalize_features_to_tag


class TestNormalizeFeaturesToTag:
    """Tests for the normalize_features_to_tag function."""

    def test_single_feature(self):
        """Test normalization of a single feature."""
        assert normalize_features_to_tag("policy-fifo") == "policy-fifo"
        assert normalize_features_to_tag("scheduling") == "scheduling"
        assert normalize_features_to_tag("feature1") == "feature1"

    def test_multiple_features_sorted(self):
        """Test that multiple features are sorted alphabetically."""
        assert normalize_features_to_tag("b,a") == "a-b"
        assert normalize_features_to_tag("policy-b,policy-a") == "policy-a-policy-b"
        assert normalize_features_to_tag("z,y,x") == "x-y-z"
        assert normalize_features_to_tag("feat3,feat1,feat2") == "feat1-feat2-feat3"

    def test_features_with_spaces(self):
        """Test that spaces are handled correctly."""
        assert normalize_features_to_tag("a, b, c") == "a-b-c"
        assert normalize_features_to_tag(" feat1 , feat2 ") == "feat1-feat2"

    def test_empty_features(self):
        """Test that empty features default to 'latest'."""
        assert normalize_features_to_tag("") == "latest"
        assert normalize_features_to_tag("   ") == "latest"
        assert normalize_features_to_tag(None) == "latest"

    def test_uppercase_to_lowercase(self):
        """Test that uppercase features are converted to lowercase."""
        assert normalize_features_to_tag("FEAT1") == "feat1"
        assert normalize_features_to_tag("Policy-A") == "policy-a"
        assert normalize_features_to_tag("FEAT1,FEAT2") == "feat1-feat2"

    def test_valid_docker_characters(self):
        """Test that valid docker tag characters are preserved."""
        assert normalize_features_to_tag("feat_with_underscore") == "feat_with_underscore"
        assert normalize_features_to_tag("feat.with.dots") == "feat.with.dots"
        assert normalize_features_to_tag("feat-with-dashes") == "feat-with-dashes"

    def test_invalid_characters_replaced(self):
        """Test that invalid docker tag characters are replaced."""
        # Spaces within features should be replaced
        assert normalize_features_to_tag("feat with space") == "feat-with-space"
        # Special characters should be replaced
        assert normalize_features_to_tag("feat@special") == "feat-special"
        assert normalize_features_to_tag("feat#hash") == "feat-hash"

    def test_leading_period_dash_removed(self):
        """Test that leading periods or dashes are removed."""
        assert normalize_features_to_tag(".feat") == "feat"
        assert normalize_features_to_tag("-feat") == "feat"

    def test_multiple_dashes_collapsed(self):
        """Test that multiple consecutive dashes are collapsed."""
        # After replacing invalid chars, multiple dashes should collapse
        result = normalize_features_to_tag("feat--with--dashes")
        assert "--" not in result

    def test_realistic_cargo_features(self):
        """Test with realistic cargo feature flag combinations."""
        # Common scheduling policies
        assert normalize_features_to_tag("scheduling-fifo") == "scheduling-fifo"
        assert normalize_features_to_tag("scheduling-lifo") == "scheduling-lifo"
        assert normalize_features_to_tag("scheduling-edf,tracing") == "scheduling-edf-tracing"
        
        # Multiple policies
        result = normalize_features_to_tag("policy-preemptive,policy-priority,metrics")
        assert result == "metrics-policy-preemptive-policy-priority"

    def test_determinism(self):
        """Test that the same features always produce the same tag."""
        features = "feat3,feat1,feat2"
        tag1 = normalize_features_to_tag(features)
        tag2 = normalize_features_to_tag(features)
        tag3 = normalize_features_to_tag(features)
        
        assert tag1 == tag2 == tag3
        assert tag1 == "feat1-feat2-feat3"

    def test_order_independence(self):
        """Test that different orderings produce the same tag."""
        tag1 = normalize_features_to_tag("a,b,c")
        tag2 = normalize_features_to_tag("c,b,a")
        tag3 = normalize_features_to_tag("b,a,c")
        
        assert tag1 == tag2 == tag3
        assert tag1 == "a-b-c"


class TestHotelLoadGenerator:
    """Tests for HotelLoadGenerator with feature-based tags."""

    def test_image_name_with_features(self):
        """Test that load generator creates correct image name with features."""
        loadgen = HotelLoadGenerator(features="policy-a,policy-b")
        assert loadgen.get_image_name() == "hotel_client_bench:policy-a-policy-b"

    def test_image_name_without_features(self):
        """Test that load generator uses 'latest' without features."""
        loadgen = HotelLoadGenerator(features=None)
        assert loadgen.get_image_name() == "hotel_client_bench:latest"
        
        loadgen2 = HotelLoadGenerator(features="")
        assert loadgen2.get_image_name() == "hotel_client_bench:latest"

    def test_container_name(self):
        """Test that container name is constant."""
        loadgen = HotelLoadGenerator(features="test")
        assert loadgen.get_container_name() == "hotel_client_bench"

    def test_network_name(self):
        """Test that network name is constant."""
        loadgen = HotelLoadGenerator(features="test")
        assert loadgen.get_network_name() == "local_hotel_network"

    def test_binary_name(self):
        """Test that binary name is constant."""
        loadgen = HotelLoadGenerator(features="test")
        assert loadgen.get_binary_name() == "hotel_client_bench"

    def test_features_preserved(self):
        """Test that features are stored correctly."""
        features = "policy-x,policy-y"
        loadgen = HotelLoadGenerator(features=features)
        assert loadgen.features == features


class TestHotelBuilder:
    """Tests for HotelBuilder with feature-based tags."""

    @patch('exp.runner.apps.hotel.subprocess.run')
    @patch('exp.runner.apps.hotel.logger')
    def test_build_with_features(self, mock_logger, mock_subprocess):
        """Test that builder creates correct docker build command with features."""
        builder = HotelBuilder()
        mock_subprocess.return_value = Mock(returncode=0)
        
        with tempfile.TemporaryDirectory() as tmpdir:
            repo_root = Path(tmpdir) / "repo"
            app_dir = Path(tmpdir) / "app"
            repo_root.mkdir()
            app_dir.mkdir()
            
            app_config_path = repo_root / "hotel.json"
            gen_config_path = repo_root / "gen_config.json"
            features = "policy-a,policy-b"
            
            # Create config files
            app_config_path.write_text("{}")
            gen_config_path.write_text("{}")
            
            builder.build(
                repo_root=repo_root,
                app_dir=app_dir,
                features=features,
                rust_log="info",
                no_cache=False,
                app_config_path=app_config_path,
                gen_config_path=gen_config_path,
            )
            
            # Check that subprocess.run was called multiple times (builder, runtime-base, and multiple binaries)
            assert mock_subprocess.call_count >= 3
            
            # Check that all calls include docker build
            all_calls = [call[0][0] for call in mock_subprocess.call_args_list]
            for cmd in all_calls:
                assert "docker" in cmd
                assert "build" in cmd
            
            # Check that builder stage has correct tag
            builder_call = all_calls[0]
            assert "-t" in builder_call
            tag_index = builder_call.index("-t") + 1
            assert builder_call[tag_index] == "hotel_builder:policy-a-policy-b"
            
            # Check that runtime-base stage has correct tag
            runtime_base_call = all_calls[1]
            assert "-t" in runtime_base_call
            tag_index = runtime_base_call.index("-t") + 1
            assert runtime_base_call[tag_index] == "hotel_runtime-base:policy-a-policy-b"
            
            # Check that at least one binary image has the correct tag
            binary_calls = all_calls[2:]
            found_loadgen_image = False
            for cmd in binary_calls:
                if "-t" in cmd:
                    tag_index = cmd.index("-t") + 1
                    image_name = cmd[tag_index]
                    if image_name == "hotel_client_bench:policy-a-policy-b":
                        found_loadgen_image = True
                    # Check build args
                    assert f"FEATURES={features}" in str(cmd)
            
            assert found_loadgen_image, "hotel_client_bench image should be built"
            
            # Check logging
            assert mock_logger.info.call_count >= 1

    @patch('exp.runner.apps.hotel.subprocess.run')
    @patch('exp.runner.apps.hotel.logger')
    def test_build_without_features(self, mock_logger, mock_subprocess):
        """Test that builder uses 'latest' tag without features."""
        builder = HotelBuilder()
        mock_subprocess.return_value = Mock(returncode=0)
        
        with tempfile.TemporaryDirectory() as tmpdir:
            repo_root = Path(tmpdir) / "repo"
            app_dir = Path(tmpdir) / "app"
            repo_root.mkdir()
            app_dir.mkdir()
            
            app_config_path = repo_root / "hotel.json"
            gen_config_path = repo_root / "gen_config.json"
            
            # Create config files
            app_config_path.write_text("{}")
            gen_config_path.write_text("{}")
            
            builder.build(
                repo_root=repo_root,
                app_dir=app_dir,
                features=None,
                rust_log="info",
                no_cache=False,
                app_config_path=app_config_path,
                gen_config_path=gen_config_path,
            )
            
            # Check that subprocess.run was called multiple times
            assert mock_subprocess.call_count >= 3
            
            # Check that builder stage uses 'latest' tag
            all_calls = [call[0][0] for call in mock_subprocess.call_args_list]
            builder_call = all_calls[0]
            tag_index = builder_call.index("-t") + 1
            assert builder_call[tag_index] == "hotel_builder:latest"
            
            # Check that at least one binary image uses 'latest' tag
            binary_calls = all_calls[2:]
            found_loadgen_image = False
            for cmd in binary_calls:
                if "-t" in cmd:
                    tag_index = cmd.index("-t") + 1
                    image_name = cmd[tag_index]
                    if image_name == "hotel_client_bench:latest":
                        found_loadgen_image = True
            
            assert found_loadgen_image, "hotel_client_bench:latest image should be built"
            
            # Check logging
            assert mock_logger.info.call_count >= 1

    @patch('exp.runner.apps.hotel.subprocess.run')
    def test_build_with_no_cache(self, mock_subprocess):
        """Test that --no-cache flag is added when requested."""
        builder = HotelBuilder()
        mock_subprocess.return_value = Mock(returncode=0)
        
        with tempfile.TemporaryDirectory() as tmpdir:
            repo_root = Path(tmpdir) / "repo"
            app_dir = Path(tmpdir) / "app"
            repo_root.mkdir()
            app_dir.mkdir()
            
            app_config_path = repo_root / "hotel.json"
            gen_config_path = repo_root / "gen_config.json"
            
            # Create config files
            app_config_path.write_text("{}")
            gen_config_path.write_text("{}")
            
            builder.build(
                repo_root=repo_root,
                app_dir=app_dir,
                features="test",
                rust_log="info",
                no_cache=True,
                app_config_path=app_config_path,
                gen_config_path=gen_config_path,
            )
            
            # Check that --no-cache is in all build commands
            all_calls = [call[0][0] for call in mock_subprocess.call_args_list]
            for cmd in all_calls:
                assert "--no-cache" in cmd

class TestHotelApp:
    """Tests for HotelApp integration with feature-based tags."""

    def test_get_image_tag(self):
        """Test that app returns correct image tag."""
        app = HotelApp()
        
        assert app.get_image_tag("policy-a,policy-b") == "policy-a-policy-b"
        assert app.get_image_tag("scheduling-fifo") == "scheduling-fifo"
        assert app.get_image_tag(None) == "latest"
        assert app.get_image_tag("") == "latest"

    def test_create_load_generator_with_features(self):
        """Test that app creates load generator with features."""
        app = HotelApp()
        features = "policy-x,policy-y"
        
        loadgen = app.create_load_generator(features=features)
        
        assert isinstance(loadgen, HotelLoadGenerator)
        assert loadgen.features == features
        assert loadgen.get_image_name() == "hotel_client_bench:policy-x-policy-y"

    def test_create_load_generator_without_features(self):
        """Test that app creates load generator without features."""
        app = HotelApp()
        
        loadgen = app.create_load_generator(features=None)
        
        assert isinstance(loadgen, HotelLoadGenerator)
        assert loadgen.features is None
        assert loadgen.get_image_name() == "hotel_client_bench:latest"

    def test_create_builder(self):
        """Test that app creates correct builder."""
        app = HotelApp()
        builder = app.create_builder()
        
        assert isinstance(builder, HotelBuilder)

    def test_consistency_between_components(self):
        """Test that all components use consistent image tags."""
        app = HotelApp()
        features = "policy-a,policy-b,policy-c"
        
        # Get tag from app
        app_tag = app.get_image_tag(features)
        
        # Get image name from load generator
        loadgen = app.create_load_generator(features=features)
        loadgen_image = loadgen.get_image_name()
        
        # They should match - loadgen uses hotel_client_bench as the image name
        assert loadgen_image == f"hotel_client_bench:{app_tag}"
        assert app_tag == "policy-a-policy-b-policy-c"


class TestEdgeCases:
    """Test edge cases and error conditions."""

    def test_empty_string_after_split(self):
        """Test features that become empty after splitting."""
        assert normalize_features_to_tag(",,,") == "latest"
        assert normalize_features_to_tag("  ,  ,  ") == "latest"

    def test_single_feature_with_commas(self):
        """Test single feature surrounded by commas."""
        assert normalize_features_to_tag(",feat,") == "feat"
        assert normalize_features_to_tag(",,feat,,") == "feat"

    def test_very_long_feature_list(self):
        """Test with many features."""
        features = ",".join([f"feat{i}" for i in range(10)])
        result = normalize_features_to_tag(features)
        
        # Should be sorted and joined
        expected = "-".join([f"feat{i}" for i in range(10)])
        assert result == expected

    def test_unicode_characters(self):
        """Test that unicode characters are handled."""
        # Unicode should be replaced with dashes
        result = normalize_features_to_tag("feat-λ")
        assert "λ" not in result  # Should be replaced

    def test_mixed_case_sorted_correctly(self):
        """Test that case-insensitive sorting works correctly."""
        result = normalize_features_to_tag("Zebra,apple,BANANA")
        # After lowercase conversion and sorting
        assert result == "apple-banana-zebra"


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
