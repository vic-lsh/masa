"""
Tests for the CPU monitoring module.

This module tests the CPUMonitor functionality including:
- Docker stats collection
- Background thread monitoring
- CSV output generation
- Parsing of docker stats output
"""

import csv
import pytest
import tempfile
import threading
import time
from pathlib import Path
from unittest.mock import Mock, patch, MagicMock

from exp.runner.cpu_monitor import CPUMonitor


class TestCPUMonitor:
    """Tests for CPUMonitor class."""

    def test_init(self):
        """Test CPUMonitor initialization."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path, poll_interval=1.0)

            assert monitor.output_path == output_path
            assert monitor.poll_interval == 1.0
            assert monitor._thread is None
            assert monitor._stats_data == []

    def test_parse_percentage(self):
        """Test percentage string parsing."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            assert monitor._parse_percentage("12.34%") == 12.34
            assert monitor._parse_percentage("0.00%") == 0.0
            assert monitor._parse_percentage("100.00%") == 100.0
            assert monitor._parse_percentage("invalid") == 0.0
            assert monitor._parse_percentage("") == 0.0

    def test_parse_memory_value(self):
        """Test memory value parsing to MB."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Test different units
            assert monitor._parse_memory_value("100MiB") == 100.0
            assert monitor._parse_memory_value("1GiB") == 1024.0
            assert monitor._parse_memory_value("512KiB") == 0.5
            assert monitor._parse_memory_value("1024B") == pytest.approx(0.0009765625, rel=1e-6)

            # Test case insensitivity
            assert monitor._parse_memory_value("100mib") == 100.0
            assert monitor._parse_memory_value("1gib") == 1024.0

            # Test with spaces
            assert monitor._parse_memory_value("100 MiB") == 100.0

            # Test invalid input
            assert monitor._parse_memory_value("invalid") == 0.0
            assert monitor._parse_memory_value("") == 0.0

    def test_parse_memory(self):
        """Test full memory string parsing."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            usage, limit = monitor._parse_memory("123.4MiB / 1.5GiB")
            assert usage == 123.4
            assert limit == 1536.0  # 1.5 * 1024

            usage, limit = monitor._parse_memory("512MiB / 2GiB")
            assert usage == 512.0
            assert limit == 2048.0

            # Test invalid input
            usage, limit = monitor._parse_memory("invalid")
            assert usage == 0.0
            assert limit == 0.0

    @patch('exp.runner.cpu_monitor.subprocess.run')
    def test_collect_stats_success(self, mock_run):
        """Test successful stats collection."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Mock docker stats output
            mock_result = Mock()
            mock_result.stdout = (
                "container1\t12.34%\t123.4MiB / 1GiB\t12.05%\n"
                "container2\t45.67%\t512MiB / 2GiB\t25.00%\n"
            )
            mock_run.return_value = mock_result

            stats = monitor._collect_stats()

            assert len(stats) == 2

            # Check first container
            assert stats[0]["container_name"] == "container1"
            assert stats[0]["cpu_percent"] == 12.34
            assert stats[0]["memory_usage_mb"] == 123.4
            assert stats[0]["memory_limit_mb"] == 1024.0
            assert stats[0]["memory_percent"] == 12.05
            assert "timestamp" in stats[0]

            # Check second container
            assert stats[1]["container_name"] == "container2"
            assert stats[1]["cpu_percent"] == 45.67
            assert stats[1]["memory_usage_mb"] == 512.0
            assert stats[1]["memory_limit_mb"] == 2048.0
            assert stats[1]["memory_percent"] == 25.00

    @patch('exp.runner.cpu_monitor.subprocess.run')
    def test_collect_stats_timeout(self, mock_run):
        """Test stats collection with timeout."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Mock timeout
            import subprocess
            mock_run.side_effect = subprocess.TimeoutExpired(cmd="docker", timeout=10.0)

            stats = monitor._collect_stats()
            assert stats == []

    @patch('exp.runner.cpu_monitor.subprocess.run')
    def test_collect_stats_error(self, mock_run):
        """Test stats collection with command error."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Mock command error
            import subprocess
            mock_run.side_effect = subprocess.CalledProcessError(
                returncode=1, cmd="docker", stderr="error"
            )

            stats = monitor._collect_stats()
            assert stats == []

    @patch('exp.runner.cpu_monitor.subprocess.run')
    def test_start_and_stop(self, mock_run):
        """Test starting and stopping the monitor."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path, poll_interval=0.1)

            # Mock successful stats collection
            mock_result = Mock()
            mock_result.stdout = "test_container\t10.0%\t100MiB / 1GiB\t10.0%\n"
            mock_run.return_value = mock_result

            # Start monitoring
            monitor.start()
            assert monitor._thread is not None
            assert monitor._thread.is_alive()

            # Let it collect some data
            time.sleep(0.3)

            # Stop monitoring
            monitor.stop()
            assert not monitor._thread.is_alive()

            # Check that CSV was created
            assert output_path.exists()

            # Read and verify CSV
            with open(output_path, 'r') as f:
                reader = csv.DictReader(f)
                rows = list(reader)
                assert len(rows) > 0
                assert rows[0]["container_name"] == "test_container"
                assert rows[0]["cpu_percent"] == "10.0"

    def test_save_stats(self):
        """Test saving stats to CSV file."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Manually add some test data
            monitor._stats_data = [
                {
                    "timestamp": 1234567890.0,
                    "container_name": "test1",
                    "cpu_percent": 10.5,
                    "memory_usage_mb": 100.0,
                    "memory_limit_mb": 1000.0,
                    "memory_percent": 10.0,
                },
                {
                    "timestamp": 1234567891.0,
                    "container_name": "test2",
                    "cpu_percent": 20.5,
                    "memory_usage_mb": 200.0,
                    "memory_limit_mb": 2000.0,
                    "memory_percent": 10.0,
                },
            ]

            monitor._save_stats()

            # Verify file was created
            assert output_path.exists()

            # Read and verify contents
            with open(output_path, 'r') as f:
                reader = csv.DictReader(f)
                rows = list(reader)

                assert len(rows) == 2
                assert rows[0]["container_name"] == "test1"
                assert float(rows[0]["cpu_percent"]) == 10.5
                assert rows[1]["container_name"] == "test2"
                assert float(rows[1]["cpu_percent"]) == 20.5

    def test_save_stats_empty(self):
        """Test saving with no stats data."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            monitor._save_stats()

            # File should not be created for empty data
            assert not output_path.exists()

    def test_start_already_running(self):
        """Test starting monitor when already running."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path, poll_interval=0.5)

            # Mock the collect stats to avoid actual docker calls
            with patch.object(monitor, '_collect_stats', return_value=[]):
                monitor.start()
                assert monitor._thread is not None
                first_thread = monitor._thread

                # Try to start again
                monitor.start()
                # Should be the same thread (not restarted)
                assert monitor._thread == first_thread

                monitor.stop()

    def test_stop_not_running(self):
        """Test stopping monitor when not running."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Should not raise an error
            monitor.stop()

    def test_thread_safety(self):
        """Test that stats data collection is thread-safe."""
        with tempfile.TemporaryDirectory() as tmpdir:
            output_path = Path(tmpdir) / "cpu_stats.csv"
            monitor = CPUMonitor(output_path=output_path)

            # Add data from multiple "threads"
            def add_data(n):
                for i in range(10):
                    with monitor._lock:
                        monitor._stats_data.append({
                            "timestamp": time.time(),
                            "container_name": f"container_{n}_{i}",
                            "cpu_percent": float(i),
                            "memory_usage_mb": 100.0,
                            "memory_limit_mb": 1000.0,
                            "memory_percent": 10.0,
                        })

            threads = [threading.Thread(target=add_data, args=(i,)) for i in range(5)]
            for t in threads:
                t.start()
            for t in threads:
                t.join()

            # Should have 50 entries (5 threads * 10 entries)
            assert len(monitor._stats_data) == 50


if __name__ == "__main__":
    pytest.main([__file__, "-v"])
