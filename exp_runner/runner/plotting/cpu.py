"""CPU utilization plotting for experiment results."""

import logging
from pathlib import Path
from typing import Optional

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

from ..container_utils import extract_service_name
from .util import get_policy_color, get_policy_display_name

logger = logging.getLogger(__name__)


def plot_cpu_utilization(
    data_dir: Path,
    output_dir: Path,
    figsize: tuple[int, int] = (12, 6),
    policies: Optional[list[str]] = None,
) -> None:
    """
    Generate CPU utilization time series plots.

    Creates one plot per service, averaging CPU usage across all replicas.

    Args:
        data_dir: Directory containing experiment output (with cpu_stats.csv files)
        output_dir: Directory to save plots
        figsize: Figure size for plots (width, height)
    """
    logger.info("Generating CPU utilization plots")

    # Find all cpu_stats.csv files
    cpu_stats_files = list(data_dir.rglob("cpu_stats.csv"))
    if not cpu_stats_files:
        logger.warning(f"No cpu_stats.csv files found in {data_dir}")
        return

    logger.info(f"Found {len(cpu_stats_files)} cpu_stats.csv files")

    # Load and combine all CPU stats data
    all_data = []
    for csv_file in cpu_stats_files:
        try:
            # Extract iteration and policy from path
            # Expected structure: data_dir / iteration / policy / cpu_stats.csv
            parts = csv_file.relative_to(data_dir).parts
            if len(parts) >= 3:
                iteration = parts[0]
                policy = parts[1]
            else:
                iteration = "0"
                policy = "unknown"

            df = pd.read_csv(csv_file)
            df["iteration"] = iteration
            df["policy"] = policy
            all_data.append(df)

        except Exception as e:
            logger.warning(f"Failed to load {csv_file}: {e}")
            continue

    if not all_data:
        logger.warning("No valid CPU stats data found")
        return

    # Combine all data
    df_all = pd.concat(all_data, ignore_index=True)
    logger.info(f"Loaded {len(df_all)} CPU stats records")

    if policies is not None:
        df_all = df_all[df_all["policy"].isin(policies)].copy()
        if df_all.empty:
            logger.warning("No CPU stats data found for requested policies")
            return

    # Add service_name column
    df_all["service_name"] = df_all["container_name"].apply(extract_service_name)

    # Filter out load generator containers
    df_all = df_all[~df_all["container_name"].str.contains("loadgen|client.*bench", case=False, regex=True)]

    # Generate plots for each service
    services = df_all["service_name"].unique()
    logger.info(f"Generating plots for {len(services)} services")

    output_dir.mkdir(parents=True, exist_ok=True)

    try:
        _save_cpu_summary_csv(df_all, output_dir)
    except Exception as e:
        logger.error(f"Failed to save CPU summary CSV: {e}")

    for service in sorted(services):
        try:
            _plot_service_cpu(df_all, service, output_dir, figsize)
        except Exception as e:
            logger.error(f"Failed to plot CPU for service {service}: {e}")

    logger.info(f"CPU utilization plots saved to {output_dir}")


def _plot_service_cpu(
    df: pd.DataFrame,
    service: str,
    output_dir: Path,
    figsize: tuple[int, int],
) -> None:
    """
    Plot CPU utilization for a single service.

    Creates time series plot with one line per policy, averaging across:
    - All replicas of the service
    - All iterations of the experiment

    Args:
        df: DataFrame with all CPU stats
        service: Service name to plot
        output_dir: Directory to save plot
        figsize: Figure size
    """
    # Filter data for this service
    df_service = df[df["service_name"] == service].copy()

    if df_service.empty:
        logger.warning(f"No data for service: {service}")
        return

    # Normalize timestamps to start at 0 for each (iteration, policy)
    # This allows us to average across iterations
    df_service["timestamp_normalized"] = 0.0
    for (iteration, policy), group in df_service.groupby(["iteration", "policy"]):
        min_ts = group["timestamp"].min()
        df_service.loc[group.index, "timestamp_normalized"] = group["timestamp"] - min_ts

    # Create time bins (e.g., 2-second intervals)
    bin_size = 2.0  # seconds
    max_time = df_service["timestamp_normalized"].max()
    time_bins = pd.interval_range(start=0, end=max_time + bin_size, freq=bin_size, closed="left")
    df_service["time_bin"] = pd.cut(df_service["timestamp_normalized"], bins=time_bins)

    # Get bin midpoints for plotting
    df_service["time_bin_mid"] = df_service["time_bin"].apply(lambda x: x.mid if pd.notna(x) else None)

    # Group by policy and time bin, compute mean CPU across replicas and iterations
    df_grouped = df_service.groupby(["policy", "time_bin_mid"], observed=True).agg({
        "cpu_percent": "mean",
    }).reset_index()

    # Plot
    fig, ax = plt.subplots(figsize=figsize)

    policies = sorted(df_grouped["policy"].unique())
    colors = _get_policy_colors(policies)

    for policy in policies:
        df_policy = df_grouped[df_grouped["policy"] == policy].sort_values("time_bin_mid")

        # Apply EWMA smoothing with alpha=0.5
        cpu_values = df_policy["cpu_percent"].values
        if len(cpu_values) > 0:
            smoothed_cpu = _apply_ewma(cpu_values, alpha=0.5)
        else:
            smoothed_cpu = cpu_values

        ax.plot(
            df_policy["time_bin_mid"],
            smoothed_cpu,
            label=get_policy_display_name(policy),
            color=colors.get(policy, None),
            linewidth=2,
            marker="o",
            markersize=3,
        )

    ax.set_xlabel("Time (seconds)", fontsize=12)
    ax.set_ylabel("CPU Utilization (%)", fontsize=12)
    ax.set_title(f"CPU Utilization - {service}", fontsize=14, fontweight="bold")
    ax.legend(loc="best", fontsize=10)
    ax.grid(True, alpha=0.3)
    ax.set_ylim(0, 100)  # CPU utilization ranges from 0 to 100%

    # Save plot
    output_file = output_dir / f"cpu_{_sanitize_filename(service)}.png"
    fig.tight_layout()
    fig.savefig(output_file, dpi=150, bbox_inches="tight")
    plt.close(fig)

    logger.info(f"Saved CPU plot for {service}: {output_file}")


def _apply_ewma(values: np.ndarray, alpha: float) -> np.ndarray:
    """
    Apply Exponential Weighted Moving Average (EWMA) smoothing.

    Args:
        values: Array of values to smooth
        alpha: Smoothing factor (0 < alpha <= 1). Higher alpha = less smoothing

    Returns:
        Smoothed array of same length
    """
    if len(values) == 0:
        return values

    smoothed = np.zeros_like(values)
    smoothed[0] = values[0]

    for i in range(1, len(values)):
        smoothed[i] = alpha * values[i] + (1 - alpha) * smoothed[i - 1]

    return smoothed


def _get_policy_colors(policies: list[str]) -> dict[str, str]:
    """
    Get colors for policies, matching the color scheme from plotting utilities.

    Args:
        policies: List of policy names

    Returns:
        Dict mapping policy name to color
    """
    result = {}
    default_colors = plt.rcParams['axes.prop_cycle'].by_key()['color']
    default_idx = 0

    for policy in policies:
        color = get_policy_color(policy)
        if color:
            result[policy] = color
        else:
            # Use default matplotlib colors for unknown policies
            result[policy] = default_colors[default_idx % len(default_colors)]
            default_idx += 1

    return result


def _sanitize_filename(name: str) -> str:
    """
    Sanitize a string for use in filename.

    Args:
        name: String to sanitize

    Returns:
        Sanitized string safe for filename
    """
    # Replace non-alphanumeric characters with underscores
    import re
    return re.sub(r"[^a-zA-Z0-9_-]", "_", name)


def plot_cpu_per_policy(
    data_dir: Path,
    output_dir: Path,
    figsize: tuple[int, int] = (14, 8),
) -> None:
    """
    Generate per-policy CPU utilization plots with subplots for each service.

    Creates one figure per policy with subplots for each service.

    Args:
        data_dir: Directory containing experiment output (with cpu_stats.csv files)
        output_dir: Directory to save plots
        figsize: Figure size for plots (width, height)
    """
    logger.info("Generating per-policy CPU utilization plots")

    # Find all cpu_stats.csv files
    cpu_stats_files = list(data_dir.rglob("cpu_stats.csv"))
    if not cpu_stats_files:
        logger.warning(f"No cpu_stats.csv files found in {data_dir}")
        return

    # Load and combine all CPU stats data
    all_data = []
    for csv_file in cpu_stats_files:
        try:
            parts = csv_file.relative_to(data_dir).parts
            if len(parts) >= 3:
                iteration = parts[0]
                policy = parts[1]
            else:
                iteration = "0"
                policy = "unknown"

            df = pd.read_csv(csv_file)
            df["iteration"] = iteration
            df["policy"] = policy
            all_data.append(df)

        except Exception as e:
            logger.warning(f"Failed to load {csv_file}: {e}")
            continue

    if not all_data:
        logger.warning("No valid CPU stats data found")
        return

    df_all = pd.concat(all_data, ignore_index=True)
    df_all["service_name"] = df_all["container_name"].apply(extract_service_name)

    # Filter out load generator containers
    df_all = df_all[~df_all["container_name"].str.contains("loadgen|client.*bench", case=False, regex=True)]

    # Get unique policies and services
    policies = sorted(df_all["policy"].unique())
    services = sorted(df_all["service_name"].unique())

    output_dir.mkdir(parents=True, exist_ok=True)

    # Create one plot per policy
    for policy in policies:
        try:
            _plot_policy_services(df_all, policy, services, output_dir, figsize)
        except Exception as e:
            logger.error(f"Failed to plot CPU for policy {policy}: {e}")

    logger.info(f"Per-policy CPU plots saved to {output_dir}")


def _plot_policy_services(
    df: pd.DataFrame,
    policy: str,
    services: list[str],
    output_dir: Path,
    figsize: tuple[int, int],
) -> None:
    """
    Plot CPU utilization for all services under a single policy.

    Args:
        df: DataFrame with all CPU stats
        policy: Policy name
        services: List of service names
        output_dir: Directory to save plot
        figsize: Figure size
    """
    df_policy = df[df["policy"] == policy].copy()

    if df_policy.empty:
        logger.warning(f"No data for policy: {policy}")
        return

    # Determine subplot layout
    n_services = len(services)
    n_cols = min(3, n_services)
    n_rows = (n_services + n_cols - 1) // n_cols

    fig, axes = plt.subplots(n_rows, n_cols, figsize=figsize, squeeze=False)
    axes_flat = axes.flatten()

    for idx, service in enumerate(services):
        ax = axes_flat[idx]
        df_service = df_policy[df_policy["service_name"] == service].copy()

        if df_service.empty:
            ax.set_visible(False)
            continue

        # Normalize timestamps
        for iteration, group in df_service.groupby("iteration"):
            min_ts = group["timestamp"].min()
            df_service.loc[group.index, "timestamp_normalized"] = group["timestamp"] - min_ts

        # Group by time and compute mean across replicas and iterations
        df_service["time_rounded"] = (df_service["timestamp_normalized"] // 2) * 2  # 2-second bins

        df_grouped = df_service.groupby("time_rounded").agg({
            "cpu_percent": "mean",
        }).reset_index()

        ax.plot(
            df_grouped["time_rounded"],
            df_grouped["cpu_percent"],
            linewidth=2,
            marker="o",
            markersize=3,
        )

        ax.set_xlabel("Time (s)", fontsize=10)
        ax.set_ylabel("CPU %", fontsize=10)
        ax.set_title(service, fontsize=11, fontweight="bold")
        ax.grid(True, alpha=0.3)
        ax.set_ylim(bottom=0)

    # Hide unused subplots
    for idx in range(n_services, len(axes_flat)):
        axes_flat[idx].set_visible(False)

    fig.suptitle(
        f"CPU Utilization - {get_policy_display_name(policy)}",
        fontsize=14,
        fontweight="bold",
    )
    fig.tight_layout()

    output_file = output_dir / f"cpu_policy_{_sanitize_filename(policy)}.png"
    fig.savefig(output_file, dpi=150, bbox_inches="tight")
    plt.close(fig)

    logger.info(f"Saved per-policy CPU plot for {policy}: {output_file}")


def _save_cpu_summary_csv(df: pd.DataFrame, output_dir: Path) -> None:
    """
    Save CPU utilization summary stats to CSV.

    Args:
        df: DataFrame with all CPU stats (must have service_name, policy columns)
        output_dir: Directory to save CSV
    """
    summary_data = []

    # Group by service and policy
    # We aggregate across all iterations and time points
    # This gives a single row per (service, policy)
    for (service, policy), group in df.groupby(["service_name", "policy"]):
        cpu_values = group["cpu_percent"]
        if cpu_values.empty:
            continue

        summary_data.append({
            "Service": service,
            "Policy": policy,
            "Mean_CPU": cpu_values.mean(),
            "Median_CPU": cpu_values.median(),
            "P90_CPU": cpu_values.quantile(0.90),
            "P99_CPU": cpu_values.quantile(0.99),
            "Max_CPU": cpu_values.max(),
            "Sample_Count": len(cpu_values)
        })

    if not summary_data:
        return

    summary_df = pd.DataFrame(summary_data)
    # Sort for better readability
    summary_df = summary_df.sort_values(["Service", "Policy"])

    output_file = output_dir / "cpu_summary.csv"
    summary_df.to_csv(output_file, index=False)
    logger.info(f"Saved CPU summary CSV to {output_file}")
