import os
from concurrent.futures import ThreadPoolExecutor, as_completed

import matplotlib
matplotlib.use('Agg')  # Use non-interactive backend for thread safety
import matplotlib.pyplot as plt
import numpy as np
from typing import Optional

# Suppress warning about too many open figures when running in parallel
# We properly close all figures, but many may be open simultaneously during parallel execution
plt.rcParams['figure.max_open_warning'] = 0

from .util import parse_args, prepare_output_dir, read_data, filter_excluded_errors


def get_policy_color(policy: str):
    """Get color for a policy. FIFO uses grey hues, prio_global uses blue hues."""
    policy_lower = policy.lower()
    if policy_lower.startswith("fifo"):
        if ",early" in policy_lower:
            return "darkgrey"
        return "grey"
    elif policy_lower.startswith("prio_global"):
        if ",early" in policy_lower:
            return "cornflowerblue"
        return "steelblue"
    return None  # Use matplotlib default color cycle


def sort_policies_by_type(policies):
    """Sort policies to group fifo first, then prio_global."""
    fifo_policies = []
    prio_policies = []
    other_policies = []
    
    for policy in policies:
        policy_lower = policy.lower()
        if policy_lower.startswith("fifo"):
            fifo_policies.append(policy)
        elif policy_lower.startswith("prio_global"):
            prio_policies.append(policy)
        else:
            other_policies.append(policy)
    
    # Sort within each group (base policy before early variant)
    fifo_policies.sort(key=lambda p: (",early" in p.lower(), p))
    prio_policies.sort(key=lambda p: (",early" in p.lower(), p))
    other_policies.sort()
    
    return fifo_policies + prio_policies + other_policies


def compute_goodput(df):
    # Filter out /ClientTimeout and /EarlyReturn errors - they are not goodput
    df_filtered = filter_excluded_errors(df)
    
    if df_filtered.empty:
        return 0.0
    
    # Count goodput based on execution time vs SLO
    df_filtered["met_slo"] = df_filtered["latency"] <= df_filtered["slo"]
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    return df_filtered["met_slo"].sum() / duration_us * s_to_us


def compute_goodput_time_series(df, bucket_seconds: float = 1.0):
    """Return per-second goodput counts for the provided request dataframe."""
    if bucket_seconds <= 0:
        raise ValueError("bucket_seconds must be positive")

    if df.empty:
        return np.array([]), np.array([])

    # Filter out /ClientTimeout and /EarlyReturn errors - they are not goodput
    df_filtered = filter_excluded_errors(df)
    
    if df_filtered.empty:
        # If all requests have /ClientTimeout or /EarlyReturn errors, return empty arrays
        start = df["start_at"].min()
        end = (df["start_at"] + df["latency"]).max()
        if end <= start:
            end = start + int(bucket_seconds * 10**6)
        bucket_us = int(bucket_seconds * 10**6)
        bins = np.arange(start, end + bucket_us, bucket_us)
        if len(bins) < 2:
            bins = np.array([start, start + bucket_us])
        counts = np.zeros(len(bins) - 1, dtype=int)
        bin_edges = bins
        times_seconds = (bin_edges[:-1] - start) / 10**6
        goodput = counts / bucket_seconds
        return times_seconds, goodput

    # Count goodput based on execution time vs SLO
    met_slo_mask = df_filtered["latency"] <= df_filtered["slo"]
    
    bucket_us = int(bucket_seconds * 10**6)
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()

    if end <= start:
        end = start + bucket_us

    bins = np.arange(start, end + bucket_us, bucket_us)
    if len(bins) < 2:
        bins = np.array([start, start + bucket_us])

    if met_slo_mask.any():
        completion_times = df_filtered.loc[met_slo_mask, "start_at"] + df_filtered.loc[met_slo_mask, "latency"]
        counts, bin_edges = np.histogram(completion_times, bins=bins)
    else:
        counts = np.zeros(len(bins) - 1, dtype=int)
        bin_edges = bins

    times_seconds = (bin_edges[:-1] - start) / 10**6
    goodput = counts / bucket_seconds

    return times_seconds, goodput


def plot_goodput_time_series(
    df,
    output_path: str,
    *,
    bucket_seconds: float = 1.0,
    title: Optional[str] = None,
) -> None:
    times, goodput = compute_goodput_time_series(df, bucket_seconds=bucket_seconds)

    fig, ax = plt.subplots(figsize=(12, 6))

    if times.size == 0:
        ax.text(0.5, 0.5, "No data", transform=ax.transAxes, ha="center", va="center")
        ax.set_xlabel("Time (seconds)")
        ax.set_ylabel("Goodput (requests / second)")
    else:
        ax.bar(
            times,
            goodput,
            width=bucket_seconds,
            align="edge",
            edgecolor="black",
            alpha=0.7,
        )
        ax.set_xlabel("Time since first request (seconds)")
        ax.set_ylabel("Goodput (requests / second)")

    if title:
        ax.set_title(title)

    ax.grid(axis="y", linestyle="--", alpha=0.7)
    fig.tight_layout()
    fig.savefig(output_path, dpi=300)
    plt.close(fig)


def _plot_policy_goodput_comparison(
    output_dir: str,
    api: str,
    policies: list,
    rps_values: list,
    policy_goodputs: dict,
) -> None:
    """Generate policy goodput comparison plot for a specific repeat and API."""
    fig, ax = plt.subplots(figsize=(12, 6))

    # Sort policies to group by type
    sorted_policies = sort_policies_by_type(policies)
    
    # set width of bars
    bar_width = 0.12
    index = np.arange(len(rps_values))

    # Create bars
    for j, policy in enumerate(sorted_policies):
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        color = get_policy_color(policy)
        bars = ax.bar(
            index + offset,
            policy_goodputs[policy],
            bar_width,
            label=policy,
            color=color,
        )

    # Add labels and title
    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("Goodput (requests meeting SLO per second)")
    ax.set_title(f"Goodput Comparison by Policy and RPS for {api} API")
    ax.set_xticks(index)
    ax.set_xticklabels([str(rps) for rps in rps_values])
    ax.legend()

    ax.grid(axis="y", linestyle="--", alpha=0.7)
    fig.tight_layout()
    fig.savefig(
        os.path.join(output_dir, f"policy_goodput_comparison_{api}.png"),
        dpi=300,
    )
    plt.close(fig)


def _plot_goodput_time_series_task(
    df,
    output_path: str,
    title: str,
) -> None:
    """Generate a single goodput time series plot."""
    policy_dir = os.path.dirname(output_path)
    os.makedirs(policy_dir, exist_ok=True)
    plot_goodput_time_series(
        df,
        output_path,
        bucket_seconds=1.0,
        title=title,
    )


def _plot_averaged_goodput(
    output_dir: str,
    api: str,
    policies: list,
    rps_values: list,
    policy_goodputs: list,
    repeats: int,
) -> None:
    """Generate averaged goodput comparison plot for a specific API."""
    fig, ax = plt.subplots(figsize=(12, 6))

    # Sort policies to group by type
    sorted_policies = sort_policies_by_type(policies)
    
    bar_width = 0.12
    index = np.arange(len(rps_values))

    for j, policy in enumerate(sorted_policies):
        average_goodput = (
            sum(np.array(policy_goodputs[i][api][policy]) for i in range(repeats))
            / repeats
        )
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        color = get_policy_color(policy)
        bars = ax.bar(
            index + offset,
            average_goodput,
            bar_width,
            label=policy,
            color=color,
        )

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("average goodput (requests meeting SLO per second)")
    ax.set_title(
        f"average goodput comparison by policy and RPS for {api} API over {repeats} runs"
    )
    ax.set_xticks(index)
    ax.set_xticklabels([str(rps) for rps in rps_values])
    ax.legend()

    ax.grid(axis="y", linestyle="--", alpha=0.7)
    fig.tight_layout()
    fig.savefig(
        os.path.join(output_dir, f"policy_goodput_comparison_{api}_averaged.png"),
        dpi=300,
    )
    plt.close(fig)


def generate_plots(args) -> None:
    prepare_output_dir(args)

    repeats, apis, policies, rps_values, results = read_data(
        args.config_dir, args.data_dir
    )

    # First, compute all policy goodputs (needed for plots)
    policy_goodputs = []
    for i in range(repeats):
        policy_goodputs.append({})
        for api in apis:
            data = results[i][api]
            policy_goodputs[i][api] = {
                policy: [compute_goodput(data[policy][rps]) for rps in rps_values]
                for policy in policies
            }

    # Generate plots in parallel
    with ThreadPoolExecutor() as executor:
        futures = []
        
        # Submit policy goodput comparison plots for each (repeat, api)
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                futures.append(
                    executor.submit(
                        _plot_policy_goodput_comparison,
                        output_dir,
                        api,
                        policies,
                        rps_values,
                        policy_goodputs[i][api],
                    )
                )
        
        # Submit goodput time series plots for each (repeat, api, policy, rps)
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                data = results[i][api]
                for policy in policies:
                    policy_dir = os.path.join(output_dir, policy)
                    for rps in rps_values:
                        df = data[policy][rps].copy()  # Copy to avoid race conditions
                        output_path = os.path.join(
                            policy_dir, f"goodput_time_series_{rps}rps_{api}.png"
                        )
                        title = f"Goodput Time Series for {api} - {policy} - {rps} RPS"
                        futures.append(
                            executor.submit(
                                _plot_goodput_time_series_task,
                                df,
                                output_path,
                                title,
                            )
                        )
        
        # Submit averaged goodput plots for each api
        output_dir = args.output_dir
        for api in apis:
            futures.append(
                executor.submit(
                    _plot_averaged_goodput,
                    output_dir,
                    api,
                    policies,
                    rps_values,
                    policy_goodputs,
                    repeats,
                )
            )
        
        # Wait for all plots to complete
        for future in as_completed(futures):
            try:
                future.result()
            except Exception as e:
                raise RuntimeError(f"Failed to generate goodput plot: {e}") from e


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)
