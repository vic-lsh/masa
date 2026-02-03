"""CPU and memory utilization monitoring for Docker containers."""

import csv
import logging
import re
import subprocess
import threading
import time
from pathlib import Path
from typing import Optional

logger = logging.getLogger(__name__)


class CPUMonitor:
    """Monitors CPU and memory utilization of Docker containers over time."""

    def __init__(
        self,
        output_path: Path,
        poll_interval: float = 2.0,
        container_prefix: Optional[str] = None,
        container_names: Optional[list[str]] = None,
    ):
        """
        Initialize CPU monitor.

        Args:
            output_path: Path to save the CPU stats CSV file
            poll_interval: Seconds between docker stats polling (default: 2.0)
            container_prefix: Optional prefix to filter containers (e.g., "mssim-exp1-abc123")
                            If provided, only containers with names starting with this prefix will be monitored
            container_names: Optional list of specific container names to monitor
                            If provided, only containers with names in this list will be monitored
        """
        self.output_path = output_path
        self.poll_interval = poll_interval
        self.container_prefix = container_prefix
        self.container_names = set(container_names) if container_names else None
        self._stop_event = threading.Event()
        self._thread: Optional[threading.Thread] = None
        self._stats_data: list[dict] = []
        self._lock = threading.Lock()

    def start(self) -> None:
        """Start monitoring Docker container stats in a background thread."""
        if self._thread is not None and self._thread.is_alive():
            logger.warning("CPU monitor already running")
            return

        self._stop_event.clear()
        self._stats_data = []
        self._thread = threading.Thread(target=self._monitor_loop, daemon=True)
        self._thread.start()

        filter_msg = []
        if self.container_prefix:
            filter_msg.append(f"prefix={self.container_prefix}")
        if self.container_names:
            filter_msg.append(f"names={list(self.container_names)[:3]}...")

        logger.info(
            f"Started CPU monitoring (poll_interval={self.poll_interval}s, {', '.join(filter_msg)})"
        )

    def stop(self) -> None:
        """Stop monitoring and save collected stats to file."""
        if self._thread is None or not self._thread.is_alive():
            logger.warning("CPU monitor not running")
            return

        self._stop_event.set()
        self._thread.join(timeout=10.0)
        self._save_stats()
        logger.info(
            f"Stopped CPU monitoring, saved {len(self._stats_data)} records to {self.output_path}"
        )

    def _monitor_loop(self) -> None:
        """Main monitoring loop that polls docker stats periodically."""
        while not self._stop_event.is_set():
            try:
                stats = self._collect_stats()
                if stats:
                    with self._lock:
                        self._stats_data.extend(stats)
            except Exception as e:
                logger.error(f"Error collecting docker stats: {e}")

            # Sleep but check stop event frequently to allow quick shutdown
            for _ in range(int(self.poll_interval * 10)):
                if self._stop_event.is_set():
                    break
                time.sleep(0.1)

    def _collect_stats(self) -> list[dict]:
        """
        Collect docker stats for all running containers.

        Returns:
            List of dicts with keys: timestamp, container_name, cpu_percent,
                                     memory_usage_mb, memory_limit_mb, memory_percent
        """
        try:
            # Use docker stats --no-stream to get a single snapshot
            # Format: container_name, cpu_percent, mem_usage, mem_limit, mem_percent
            cmd = [
                "docker",
                "stats",
                "--no-stream",
                "--no-trunc",
                "--format",
                "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}\t{{.MemPerc}}",
            ]

            result = subprocess.run(
                cmd, capture_output=True, text=True, timeout=10.0, check=True
            )

            timestamp = time.time()
            stats = []

            for line in result.stdout.strip().split("\n"):
                if not line:
                    continue

                parts = line.split("\t")
                if len(parts) != 4:
                    continue

                container_name, cpu_str, mem_usage_str, mem_percent_str = parts

                # Filter by container prefix if specified
                if self.container_prefix and not container_name.startswith(
                    self.container_prefix
                ):
                    continue

                # Filter by specific container names if specified
                if (
                    self.container_names is not None
                    and container_name not in self.container_names
                ):
                    continue

                # Parse CPU percentage (e.g., "12.34%" -> 12.34)
                cpu_percent = self._parse_percentage(cpu_str)

                # Parse memory usage (e.g., "123.4MiB / 1.5GiB" -> usage_mb, limit_mb)
                memory_usage_mb, memory_limit_mb = self._parse_memory(mem_usage_str)

                # Parse memory percentage (e.g., "8.23%" -> 8.23)
                memory_percent = self._parse_percentage(mem_percent_str)

                stats.append(
                    {
                        "timestamp": timestamp,
                        "container_name": container_name,
                        "cpu_percent": cpu_percent,
                        "memory_usage_mb": memory_usage_mb,
                        "memory_limit_mb": memory_limit_mb,
                        "memory_percent": memory_percent,
                    }
                )

            return stats

        except subprocess.TimeoutExpired:
            logger.warning("docker stats command timed out")
            return []
        except subprocess.CalledProcessError as e:
            logger.error(f"docker stats failed: {e.stderr}")
            return []
        except Exception as e:
            logger.error(f"Unexpected error in _collect_stats: {e}")
            return []

    def _parse_percentage(self, percent_str: str) -> float:
        """Parse percentage string like '12.34%' to float 12.34."""
        try:
            return float(percent_str.rstrip("%"))
        except (ValueError, AttributeError):
            return 0.0

    def _parse_memory(self, mem_str: str) -> tuple[float, float]:
        """
        Parse memory string like '123.4MiB / 1.5GiB' to (usage_mb, limit_mb).

        Returns:
            Tuple of (usage_mb, limit_mb)
        """
        try:
            parts = mem_str.split(" / ")
            if len(parts) != 2:
                return 0.0, 0.0

            usage_mb = self._parse_memory_value(parts[0])
            limit_mb = self._parse_memory_value(parts[1])
            return usage_mb, limit_mb

        except Exception:
            return 0.0, 0.0

    def _parse_memory_value(self, value_str: str) -> float:
        """
        Parse memory value like '123.4MiB' or '1.5GiB' to MB.

        Returns:
            Memory value in MB
        """
        # Match number followed by unit
        match = re.match(r"([\d.]+)\s*([A-Za-z]+)", value_str.strip())
        if not match:
            return 0.0

        value = float(match.group(1))
        unit = match.group(2).upper()

        # Convert to MB
        if unit in ("B", "BYTES"):
            return value / (1024 * 1024)
        elif unit in ("KB", "KIB"):
            return value / 1024
        elif unit in ("MB", "MIB"):
            return value
        elif unit in ("GB", "GIB"):
            return value * 1024
        elif unit in ("TB", "TIB"):
            return value * 1024 * 1024
        else:
            logger.warning(f"Unknown memory unit: {unit}")
            return value

    def _save_stats(self) -> None:
        """Save collected stats to CSV file."""
        if not self._stats_data:
            logger.warning("No stats data to save")
            return

        try:
            self.output_path.parent.mkdir(parents=True, exist_ok=True)

            with open(self.output_path, "w", newline="") as f:
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

                with self._lock:
                    writer.writerows(self._stats_data)

            logger.info(
                f"Saved {len(self._stats_data)} stats records to {self.output_path}"
            )

        except Exception as e:
            logger.error(f"Failed to save stats to {self.output_path}: {e}")
