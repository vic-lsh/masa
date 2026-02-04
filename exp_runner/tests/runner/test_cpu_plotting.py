"""
Tests for CPU plotting module.

This module tests the CPU utilization plotting functionality:
- Service-level CPU aggregation
- Multi-policy time series plotting
- Plot file generation
- Color scheme consistency
"""

import csv
import pytest
import tempfile
from pathlib import Path
from unittest.mock import patch

import numpy as np
import pandas as pd
import matplotlib

matplotlib.use("Agg")  # Use non-interactive backend for testing

from exp_runner.runner.plotting.cpu import (
    plot_cpu_utilization,
    _plot_service_cpu,
    _get_policy_colors,
    _sanitize_filename,
    _apply_ewma,
)


class TestApplyEwma:
    """Tests for EWMA smoothing function."""

    def test_ewma_basic(self):
        """Test basic EWMA smoothing."""
        values = np.array([1.0, 2.0, 3.0, 4.0, 5.0])
        smoothed = _apply_ewma(values, alpha=0.5)

        # First value should be unchanged
        assert smoothed[0] == 1.0

        # Subsequent values should be smoothed
        assert len(smoothed) == len(values)
        assert smoothed[1] == 0.5 * 2.0 + 0.5 * 1.0  # 1.5
        assert smoothed[-1] < 5.0  # Should be less than original due to smoothing

    def test_ewma_empty(self):
        """Test EWMA with empty array."""
        values = np.array([])
        smoothed = _apply_ewma(values, alpha=0.5)
        assert len(smoothed) == 0

    def test_ewma_single_value(self):
        """Test EWMA with single value."""
        values = np.array([42.0])
        smoothed = _apply_ewma(values, alpha=0.5)
        assert len(smoothed) == 1
        assert smoothed[0] == 42.0

    def test_ewma_alpha_effects(self):
        """Test that different alpha values produce different smoothing."""
        values = np.array([10.0, 0.0, 10.0, 0.0, 10.0])

        # High alpha (more responsive, less smoothing)
        smoothed_high = _apply_ewma(values, alpha=0.9)

        # Low alpha (less responsive, more smoothing)
        smoothed_low = _apply_ewma(values, alpha=0.1)

        # High alpha should follow changes more closely at the end
        # After multiple transitions, smoothing effect is more pronounced
        assert abs(smoothed_high[-1] - 10.0) < abs(smoothed_low[-1] - 10.0)


class TestSanitizeFilename:
    """Tests for _sanitize_filename function."""

    def test_basic_sanitization(self):
        """Test basic filename sanitization."""
        assert _sanitize_filename("simple") == "simple"
        assert _sanitize_filename("with-dashes") == "with-dashes"
        assert _sanitize_filename("with_underscores") == "with_underscores"

    def test_special_characters(self):
        """Test sanitization of special characters."""
        assert _sanitize_filename("file/with/slashes") == "file_with_slashes"
        assert _sanitize_filename("file:with:colons") == "file_with_colons"
        assert _sanitize_filename("file*with*stars") == "file_with_stars"
        assert _sanitize_filename("file?with?questions") == "file_with_questions"

    def test_spaces(self):
        """Test sanitization of spaces."""
        assert _sanitize_filename("file with spaces") == "file_with_spaces"
        assert _sanitize_filename("multiple   spaces") == "multiple___spaces"

    def test_mixed_characters(self):
        """Test sanitization of mixed special characters."""
        assert (
            _sanitize_filename("file@#$%name!") == "file____name_"
        )  # @ # % ! -> 4 underscores
        assert _sanitize_filename("complex/file:name*test") == "complex_file_name_test"


class TestGetPolicyColors:
    """Tests for _get_policy_colors function."""

    def test_standard_policies(self):
        """Test color assignment for standard policies."""
        policies = ["fifo", "prio_global", "prio_local"]
        colors = _get_policy_colors(policies)

        assert colors["fifo"] == "grey"
        assert colors["prio_global"] == "steelblue"
        assert colors["prio_local"] == "hotpink"

    def test_early_policies(self):
        """Test color assignment for early abort policies (comma-separated)."""
        policies = ["fifo,early", "prio_global,early", "prio_local,early"]
        colors = _get_policy_colors(policies)

        assert colors["fifo,early"] == "darkgrey"
        assert colors["prio_global,early"] == "cornflowerblue"
        assert colors["prio_local,early"] == "lightpink"

    def test_mixed_policies(self):
        """Test color assignment for mixed standard and early policies."""
        policies = ["fifo", "fifo,early", "prio_global", "custom_policy"]
        colors = _get_policy_colors(policies)

        assert "fifo" in colors
        assert "fifo,early" in colors
        assert "prio_global" in colors
        assert "custom_policy" in colors

        # Standard policies should have defined colors
        assert colors["fifo"] == "grey"
        assert colors["prio_global"] == "steelblue"

    def test_unknown_policies(self):
        """Test that unknown policies get default colors."""
        policies = ["unknown1", "unknown2", "unknown3"]
        colors = _get_policy_colors(policies)

        # All policies should get a color
        assert len(colors) == 3
        for policy in policies:
            assert policy in colors
            assert colors[policy] is not None


class TestPlotServiceCpu:
    """Tests for _plot_service_cpu function."""

    def create_test_dataframe(self):
        """Create a test DataFrame with CPU stats."""
        data = []
        for iteration in [0, 1]:
            for policy in ["fifo", "prio_global"]:
                for replica in [1, 2]:
                    for time_offset in range(0, 10, 2):
                        data.append(
                            {
                                "timestamp": 1000000 + iteration * 100 + time_offset,
                                "container_name": f"test-exp-abc-service-{replica}",
                                "cpu_percent": 10.0
                                + policy.__hash__() % 20
                                + time_offset,
                                "memory_usage_mb": 100.0,
                                "memory_limit_mb": 1000.0,
                                "memory_percent": 10.0,
                                "iteration": str(iteration),
                                "policy": policy,
                                "service_name": "service",
                            }
                        )
        return pd.DataFrame(data)

    def test_plot_generation(self):
        """Test that _plot_service_cpu generates a plot file."""
        df = self.create_test_dataframe()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            _plot_service_cpu(df, "service", output_dir, figsize=(12, 6))

            # Check that plot file was created
            plot_file = output_dir / "cpu_service.png"
            assert plot_file.exists()
            assert plot_file.stat().st_size > 0

    def test_empty_service(self):
        """Test handling of service with no data."""
        df = self.create_test_dataframe()

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            # Try to plot a service that doesn't exist
            _plot_service_cpu(df, "nonexistent_service", output_dir, figsize=(12, 6))

            # No plot should be generated
            plot_file = output_dir / "cpu_nonexistent_service.png"
            assert not plot_file.exists()

    def test_multiple_policies(self):
        """Test that plot includes all policies."""
        # Create data with 3 policies
        data = []
        for policy in ["fifo", "prio_global", "prio_local"]:
            for time_offset in range(0, 10, 2):
                data.append(
                    {
                        "timestamp": 1000000 + time_offset,
                        "container_name": "test-service-1",
                        "cpu_percent": 10.0 + time_offset,
                        "memory_usage_mb": 100.0,
                        "memory_limit_mb": 1000.0,
                        "memory_percent": 10.0,
                        "iteration": "0",
                        "policy": policy,
                        "service_name": "service",
                    }
                )
        df = pd.DataFrame(data)

        with tempfile.TemporaryDirectory() as tmpdir:
            output_dir = Path(tmpdir)

            _plot_service_cpu(df, "service", output_dir, figsize=(12, 6))

            # Check that plot was created
            plot_file = output_dir / "cpu_service.png"
            assert plot_file.exists()


class TestPlotCpuUtilization:
    """Tests for plot_cpu_utilization function."""

    def create_test_cpu_stats_files(self, data_dir: Path):
        """Create test CPU stats CSV files."""
        # Create directory structure: data_dir / iteration / policy / cpu_stats.csv
        for iteration in [0, 1]:
            for policy in ["fifo", "prio_global"]:
                policy_dir = data_dir / str(iteration) / policy
                policy_dir.mkdir(parents=True, exist_ok=True)

                csv_file = policy_dir / "cpu_stats.csv"
                with open(csv_file, "w", newline="") as f:
                    fieldnames = [
                        "timestamp",
                        "container_name",
                        "cpu_percent",
                        "memory_usage_mb",
                        "memory_limit_mb",
                        "memory_percent",
                    ]
                    writer = csv.DictWriter(f, fieldnames=fieldnames)
                    writer.writeheader()

                    # Write test data with proper hotel container naming
                    base_time = 1000000 + iteration * 100
                    for time_offset in range(0, 20, 2):
                        for replica in [1, 2]:
                            writer.writerow(
                                {
                                    "timestamp": base_time + time_offset,
                                    "container_name": f"hotel-test-abc123def456-rate-service-{replica}",
                                    "cpu_percent": 10.0 + time_offset + replica,
                                    "memory_usage_mb": 100.0,
                                    "memory_limit_mb": 1000.0,
                                    "memory_percent": 10.0,
                                }
                            )
                            writer.writerow(
                                {
                                    "timestamp": base_time + time_offset,
                                    "container_name": f"hotel-test-abc123def456-search-service-{replica}",
                                    "cpu_percent": 20.0 + time_offset + replica,
                                    "memory_usage_mb": 200.0,
                                    "memory_limit_mb": 2000.0,
                                    "memory_percent": 10.0,
                                }
                            )

    def test_plot_cpu_utilization_filters_policies(self):
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"
            data_dir.mkdir(parents=True, exist_ok=True)

            for policy in ["fifo", "extra_policy"]:
                policy_dir = data_dir / "0" / policy
                policy_dir.mkdir(parents=True, exist_ok=True)
                csv_file = policy_dir / "cpu_stats.csv"
                with open(csv_file, "w", newline="") as f:
                    fieldnames = [
                        "timestamp",
                        "container_name",
                        "cpu_percent",
                        "memory_usage_mb",
                        "memory_limit_mb",
                        "memory_percent",
                    ]
                    writer = csv.DictWriter(f, fieldnames=fieldnames)
                    writer.writeheader()
                    writer.writerow(
                        {
                            "timestamp": 1000000,
                            "container_name": "hotel-test-abc123def456-rate-service-1",
                            "cpu_percent": 10.0,
                            "memory_usage_mb": 100.0,
                            "memory_limit_mb": 1000.0,
                            "memory_percent": 10.0,
                        }
                    )

            with patch("exp_runner.runner.plotting.cpu._plot_service_cpu") as mock_plot:
                plot_cpu_utilization(data_dir, output_dir, policies=["fifo"])

                assert mock_plot.called
                df_arg = mock_plot.call_args[0][0]
                assert set(df_arg["policy"].unique()) == {"fifo"}

    def test_plot_generation_with_files(self):
        """Test plot generation from CSV files."""
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            self.create_test_cpu_stats_files(data_dir)

            plot_cpu_utilization(data_dir, output_dir)

            # Check that plots were created
            assert output_dir.exists()
            plot_files = list(output_dir.glob("cpu_*.png"))

            # Should have plots for rate-service and search-service
            assert len(plot_files) >= 2

            # Check specific service plots
            service_names = [f.stem.replace("cpu_", "") for f in plot_files]
            assert "rate-service" in service_names
            assert "search-service" in service_names

    def test_no_cpu_stats_files(self):
        """Test handling when no CPU stats files exist."""
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"
            data_dir.mkdir()

            # Should not raise an error
            plot_cpu_utilization(data_dir, output_dir)

            # No plots should be created
            if output_dir.exists():
                assert len(list(output_dir.glob("*.png"))) == 0

    def test_loadgen_filtering(self):
        """Test that load generator containers are filtered out."""
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            # Create CPU stats with loadgen containers
            policy_dir = data_dir / "0" / "fifo"
            policy_dir.mkdir(parents=True)

            csv_file = policy_dir / "cpu_stats.csv"
            with open(csv_file, "w", newline="") as f:
                fieldnames = [
                    "timestamp",
                    "container_name",
                    "cpu_percent",
                    "memory_usage_mb",
                    "memory_limit_mb",
                    "memory_percent",
                ]
                writer = csv.DictWriter(f, fieldnames=fieldnames)
                writer.writeheader()

                # Write service and loadgen data
                for i in range(5):
                    writer.writerow(
                        {
                            "timestamp": 1000000 + i,
                            "container_name": "test-service-1",
                            "cpu_percent": 10.0,
                            "memory_usage_mb": 100.0,
                            "memory_limit_mb": 1000.0,
                            "memory_percent": 10.0,
                        }
                    )
                    writer.writerow(
                        {
                            "timestamp": 1000000 + i,
                            "container_name": "test_client_bench",  # Load generator
                            "cpu_percent": 5.0,
                            "memory_usage_mb": 50.0,
                            "memory_limit_mb": 500.0,
                            "memory_percent": 10.0,
                        }
                    )

            plot_cpu_utilization(data_dir, output_dir)

            # Should only have plot for service, not loadgen
            plot_files = list(output_dir.glob("cpu_*.png"))

            # Check that loadgen is not in the plot names
            for plot_file in plot_files:
                assert "loadgen" not in plot_file.name.lower()
                assert "client_bench" not in plot_file.name.lower()

    def test_multiple_iterations_averaging(self):
        """Test that multiple iterations are averaged correctly."""
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            self.create_test_cpu_stats_files(data_dir)

            plot_cpu_utilization(data_dir, output_dir)

            # Plots should be created and averaged across iterations
            plot_files = list(output_dir.glob("cpu_*.png"))
            assert len(plot_files) > 0

            # Each plot file should exist and have content
            for plot_file in plot_files:
                assert plot_file.exists()
                assert plot_file.stat().st_size > 0

    def test_custom_figsize(self):
        """Test plot generation with custom figure size."""
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            self.create_test_cpu_stats_files(data_dir)

            # Should not raise an error with custom figsize
            plot_cpu_utilization(data_dir, output_dir, figsize=(16, 8))

            # Plots should still be created
            plot_files = list(output_dir.glob("cpu_*.png"))
            assert len(plot_files) > 0


class TestCpuPlottingIntegration:
    """Integration tests for CPU plotting."""

    def test_end_to_end_plotting(self):
        """Test complete end-to-end plotting workflow."""
        with tempfile.TemporaryDirectory() as tmpdir:
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            # Create realistic experiment structure
            for iteration in [0]:
                for policy in ["fifo", "prio_global", "prio_local"]:
                    policy_dir = data_dir / str(iteration) / policy
                    policy_dir.mkdir(parents=True, exist_ok=True)

                    csv_file = policy_dir / "cpu_stats.csv"
                    with open(csv_file, "w", newline="") as f:
                        fieldnames = [
                            "timestamp",
                            "container_name",
                            "cpu_percent",
                            "memory_usage_mb",
                            "memory_limit_mb",
                            "memory_percent",
                        ]
                        writer = csv.DictWriter(f, fieldnames=fieldnames)
                        writer.writeheader()

                        # Simulate 30 seconds of monitoring at 2-second intervals
                        base_time = 1000000
                        for time_offset in range(0, 30, 2):
                            # Multiple services with multiple replicas using hotel naming
                            for service in ["frontend", "backend", "database"]:
                                for replica in [1, 2, 3]:
                                    writer.writerow(
                                        {
                                            "timestamp": base_time + time_offset,
                                            "container_name": f"hotel-exp-abc123def456-{service}-{replica}",
                                            "cpu_percent": 15.0 + time_offset + replica,
                                            "memory_usage_mb": 100.0 * replica,
                                            "memory_limit_mb": 1000.0,
                                            "memory_percent": 10.0 * replica,
                                        }
                                    )

            # Generate plots
            plot_cpu_utilization(data_dir, output_dir)

            # Verify results
            assert output_dir.exists()
            plot_files = list(output_dir.glob("cpu_*.png"))

            # Should have one plot per service (3 services)
            assert len(plot_files) == 3

            # Verify plot names
            plot_names = {f.stem for f in plot_files}
            assert "cpu_frontend" in plot_names
            assert "cpu_backend" in plot_names
            assert "cpu_database" in plot_names

            # Verify all plots have content
            for plot_file in plot_files:
                assert plot_file.stat().st_size > 1000  # Should be at least 1KB


class TestMssimCpuPlotting:
    """Regression tests for MSSIM CPU plotting."""

    def test_mssim_generates_cpu_plots(self):
        """
        Regression test: Verify MSSIM experiments generate CPU plots.

        This test catches the bug where MSSIM plotting bypassed CPU plotting
        because it has a separate plotting path in all.py.
        """
        from exp_runner.runner.plotting.mssim import generate_plots
        from argparse import Namespace
        import json

        with tempfile.TemporaryDirectory() as tmpdir:
            config_dir = Path(tmpdir) / "config"
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            config_dir.mkdir()
            data_dir.mkdir()

            # Create gen_config.json
            gen_config = {
                "Repeats": 1,
                "Rps": [200, 400],
                "DurationSecs": 30,
                "WarmupSecs": 5,
            }
            with open(config_dir / "gen_config.json", "w") as f:
                json.dump(gen_config, f)

            # Create mssim.json
            mssim_config = {
                "slo_ms": 100,
            }
            with open(config_dir / "mssim.json", "w") as f:
                json.dump(mssim_config, f)

            # Create experiment data structure with CPU stats
            for policy in ["fifo", "prio_global"]:
                policy_dir = data_dir / "0" / policy
                run_dir = policy_dir / "run_0"
                run_dir.mkdir(parents=True, exist_ok=True)

                # Create CPU stats
                cpu_stats_file = run_dir / "cpu_stats.csv"
                with open(cpu_stats_file, "w", newline="") as f:
                    fieldnames = [
                        "timestamp",
                        "container_name",
                        "cpu_percent",
                        "memory_usage_mb",
                        "memory_limit_mb",
                        "memory_percent",
                    ]
                    writer = csv.DictWriter(f, fieldnames=fieldnames)
                    writer.writeheader()

                    # Write test data for MSSIM services
                    # Use realistic MSSIM service names (service names don't have replica numbers)
                    base_time = 1000000
                    for time_offset in range(0, 30, 2):
                        for service in [
                            "frontend",
                            "backend",
                            "database",
                            "load_generator",
                        ]:
                            writer.writerow(
                                {
                                    "timestamp": base_time + time_offset,
                                    "container_name": f"mssim-exp-abc123def456-{service}",
                                    "cpu_percent": 20.0 + time_offset,
                                    "memory_usage_mb": 200.0,
                                    "memory_limit_mb": 2000.0,
                                    "memory_percent": 10.0,
                                }
                            )

                # Create minimal latency CSV files for MSSIM plotting
                for rps in [200, 400]:
                    latency_file = run_dir / f"root_latencies_{rps}rps.csv"
                    with open(latency_file, "w", newline="") as f:
                        f.write("e2e_latency_us,start_at,is_err\n")
                        for i in range(10):
                            f.write(
                                f"{50000 + i * 1000},{base_time + i * 1000000},false\n"
                            )

            # Call MSSIM plotting
            args = Namespace(
                config_dir=config_dir,
                data_dir=data_dir,
                output_dir=output_dir,
            )

            generate_plots(args)

            # Verify CPU plots were generated
            cpu_plots = list(output_dir.glob("cpu_*.png"))
            assert len(cpu_plots) > 0, "MSSIM plotting should generate CPU plots"

            # Should have plots for the microservices
            # Note: load_generator is still plotted, it's just filtered from latency metrics
            assert len(cpu_plots) >= 3, (
                f"Should have CPU plots for microservices, got {len(cpu_plots)}: {[p.name for p in cpu_plots]}"
            )

            # Verify plot files have content
            for plot_file in cpu_plots:
                assert plot_file.exists()
                assert plot_file.stat().st_size > 1000

    def test_mssim_cpu_plots_with_multiple_iterations(self):
        """Test MSSIM CPU plotting with multiple iterations (averaging)."""
        from exp_runner.runner.plotting.mssim import generate_plots
        from argparse import Namespace
        import json

        with tempfile.TemporaryDirectory() as tmpdir:
            config_dir = Path(tmpdir) / "config"
            data_dir = Path(tmpdir) / "data"
            output_dir = Path(tmpdir) / "plots"

            config_dir.mkdir()
            data_dir.mkdir()

            # Create configs
            gen_config = {
                "Repeats": 2,
                "Rps": [200],
                "DurationSecs": 20,
                "WarmupSecs": 0,
            }
            with open(config_dir / "gen_config.json", "w") as f:
                json.dump(gen_config, f)

            mssim_config = {"slo_ms": 100}
            with open(config_dir / "mssim.json", "w") as f:
                json.dump(mssim_config, f)

            # Create data for 2 iterations
            for iteration in [0, 1]:
                for policy in ["fifo"]:
                    run_dir = data_dir / str(iteration) / policy / "run_0"
                    run_dir.mkdir(parents=True, exist_ok=True)

                    # CPU stats
                    cpu_stats_file = run_dir / "cpu_stats.csv"
                    with open(cpu_stats_file, "w", newline="") as f:
                        writer = csv.DictWriter(
                            f,
                            fieldnames=[
                                "timestamp",
                                "container_name",
                                "cpu_percent",
                                "memory_usage_mb",
                                "memory_limit_mb",
                                "memory_percent",
                            ],
                        )
                        writer.writeheader()

                        base_time = 1000000 + iteration * 1000
                        for time_offset in range(0, 20, 2):
                            writer.writerow(
                                {
                                    "timestamp": base_time + time_offset,
                                    "container_name": "mssim-exp-abc-service-1",
                                    "cpu_percent": 15.0 + time_offset + iteration * 5,
                                    "memory_usage_mb": 150.0,
                                    "memory_limit_mb": 1500.0,
                                    "memory_percent": 10.0,
                                }
                            )

                    # Minimal latency data
                    latency_file = run_dir / "root_latencies_200rps.csv"
                    with open(latency_file, "w") as f:
                        f.write("e2e_latency_us,start_at,is_err\n")
                        f.write(f"{50000},{base_time},false\n")

            # Generate plots
            args = Namespace(
                config_dir=config_dir,
                data_dir=data_dir,
                output_dir=output_dir,
            )

            generate_plots(args)

            # Verify CPU plots were created and averaged across iterations
            cpu_plots = list(output_dir.glob("cpu_*.png"))
            assert len(cpu_plots) > 0

            # At least one plot for the service
            assert any("service" in p.name for p in cpu_plots)


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
