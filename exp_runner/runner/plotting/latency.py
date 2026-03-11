import os
from concurrent.futures import ThreadPoolExecutor, as_completed

import matplotlib
import numpy as np
import pandas as pd
import seaborn as sns

from .util import (
    filter_excluded_errors,
    get_plot_worker_count,
    get_policy_color,
    get_policy_display_name,
    parse_args,
    PlotData,
    prepare_output_dir,
    read_data,
)

matplotlib.use("Agg")  # Use non-interactive backend for thread safety
import matplotlib.pyplot as plt

# Suppress warning about too many open figures when running in parallel
# We properly close all figures, but many may be open simultaneously during parallel execution
plt.rcParams["figure.max_open_warning"] = 0


MS_TO_US = 10**3


def _plot_latency_cdf(
    output_dir: str, api: str, rps: int, policies: list, data: dict
) -> None:
    """Generate CDF plot for latency distribution."""
    fig, ax = plt.subplots(figsize=(12, 8))

    for policy in policies:
        df = data[policy][rps]
        # Filter out excluded errors for meaningful latency analysis
        df_filtered = filter_excluded_errors(df)

        if df_filtered.empty:
            continue

        latencies = np.sort(df_filtered["latency"].to_numpy() / MS_TO_US)
        percentiles = np.linspace(0, 100, len(latencies))

        color = get_policy_color(policy)
        ax.plot(
            latencies, percentiles, label=get_policy_display_name(policy), color=color
        )

    # Add labels and title
    ax.set_xlabel("Latency (milliseconds)")
    ax.set_ylabel("Percentile (%)")
    ax.set_title(f"Latency Distribution for {api} API - {rps} RPS")
    ax.grid(True, alpha=0.3)
    ax.legend()
    # Save the plot
    fig.savefig(f"{output_dir}/latency_cdf_{rps}rps_{api}.png", dpi=300)
    plt.close(fig)


def _plot_latency_histogram(
    output_dir: str, api: str, rps: int, policy: str, df
) -> None:
    """Generate histogram plot for latency distribution."""
    fig, ax = plt.subplots(figsize=(12, 8))
    # Filter out excluded errors for meaningful latency analysis
    df_filtered = filter_excluded_errors(df)
    if df_filtered.empty:
        plt.close(fig)
        return

    df_plot = df_filtered.assign(
        latency_ms=df_filtered["latency"] / MS_TO_US,
        met_slo=df_filtered["latency"] <= df_filtered["slo"],
    )

    sns.histplot(
        data=df_plot,
        x="latency_ms",
        hue="met_slo",
        hue_order=[True, False],
        ax=ax,
    )

    # Add labels and title
    ax.set_xlabel("Latency (milliseconds)")
    ax.set_title(
        f"Latency Histogram for {api} API - {get_policy_display_name(policy)} - {rps} RPS"
    )
    dir = os.path.join(output_dir, policy)
    os.makedirs(dir, exist_ok=True)
    fig.savefig(
        os.path.join(dir, f"latency_histogram_{rps}rps_{api}.png"),
        dpi=300,
    )
    plt.close(fig)


def _plot_p99_latency(
    output_dir: str,
    api: str,
    policies: list,
    rps_values: list,
    data: dict,
    max_y: float,
) -> None:
    """Generate p99 latency plot for a specific repeat and API."""
    fig, ax = plt.subplots(figsize=(12, 6))
    for policy in policies:
        p99_values = []
        for rps in rps_values:
            df = data[policy][rps]
            # Filter out excluded errors for meaningful latency analysis
            df_filtered = filter_excluded_errors(df)
            if df_filtered.empty:
                p99_latency = np.nan
            else:
                p99_latency = df_filtered["latency"].quantile(0.99) / MS_TO_US
            p99_values.append(p99_latency)
        color = get_policy_color(policy)
        ax.plot(
            rps_values,
            p99_values,
            "o-",
            label=get_policy_display_name(policy),
            color=color,
        )

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("p99 latency (milliseconds)")
    ax.set_title(f"p99 latency by policy and RPS for {api} API")
    ax.grid(True, alpha=0.3)
    ax.legend()
    ax.set_ylim(top=max_y)
    fig.savefig(f"{output_dir}/p99_latency_rps_{api}.png", dpi=300)
    plt.close(fig)


def _plot_averaged_percentile_latency(
    output_dir: str,
    api: str,
    percentile: float,
    policies: list,
    rps_values: list,
    results: list,
    repeats: int,
) -> None:
    """Generate averaged percentile latency plot."""
    # Get SLO from first repeat, first policy, first rps for max_y calculation
    slo = results[0][api][policies[0]][rps_values[0]]["slo"].max() / MS_TO_US
    max_y = slo * 4

    fig, ax = plt.subplots(figsize=(12, 6))
    for policy in policies:
        averaged_percentile = np.zeros(len(rps_values))
        for i in range(repeats):
            data = results[i][api]
            percentile_values = []
            for rps in rps_values:
                df = data[policy][rps]
                # Filter out excluded errors for meaningful latency analysis
                df_filtered = filter_excluded_errors(df)
                if df_filtered.empty:
                    percentile_latency = np.nan
                else:
                    percentile_latency = (
                        df_filtered["latency"].quantile(percentile) / MS_TO_US
                    )
                percentile_values.append(percentile_latency)
            averaged_percentile += np.array(percentile_values)
        color = get_policy_color(policy)
        ax.plot(
            rps_values,
            averaged_percentile / repeats,
            "o-",
            label=get_policy_display_name(policy),
            color=color,
        )

    p = int(percentile * 100)
    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel(f"average p{p} latency (milliseconds)")
    ax.set_title(f"p{p} latency by policy and RPS for {api} API")
    ax.grid(True, alpha=0.3)
    ax.legend()
    ax.set_ylim(bottom=0, top=max_y)
    fig.savefig(
        f"{output_dir}/p{p}_latency_rps_{api}.png",
        dpi=300,
    )
    plt.close(fig)


def _save_latency_summary_csv(
    output_dir: str,
    api: str,
    policies: list,
    rps_values: list,
    results: list,
    repeats: int,
) -> None:
    """Generate averaged latency summary CSV."""
    percentiles = [0.50, 0.90, 0.95, 0.99]
    summary_data = []

    for policy in policies:
        for rps in rps_values:
            # Initialize accumulators for this policy/rps combination
            perc_acc = {p: [] for p in percentiles}

            for i in range(repeats):
                data = results[i][api]
                if policy not in data or rps not in data[policy]:
                    continue

                df = data[policy][rps]
                df_filtered = filter_excluded_errors(df)

                if not df_filtered.empty:
                    for p in percentiles:
                        perc_acc[p].append(df_filtered["latency"].quantile(p))
                        perc_acc[p][-1] /= MS_TO_US

            row = {"API": api, "Policy": policy, "RPS": rps}
            for p in percentiles:
                col_name = f"p{int(p * 100)}"
                values = perc_acc[p]
                if values:
                    row[col_name] = sum(values) / len(values)
                else:
                    row[col_name] = np.nan
            summary_data.append(row)

    if summary_data:
        df = pd.DataFrame(summary_data)
        # Sort for better readability
        if not df.empty:
            df = df.sort_values(by=["Policy", "RPS"])
        output_path = os.path.join(output_dir, f"latency_summary_{api}.csv")
        df.to_csv(output_path, index=False)


def generate_plots(args, plot_data: PlotData | None = None) -> None:
    prepare_output_dir(args)

    if plot_data is None:
        repeats, apis, policies, rps_values, results = read_data(
            args.config_dir, args.data_dir
        )
    else:
        repeats = plot_data.repeats
        apis = plot_data.apis
        policies = plot_data.policies
        rps_values = plot_data.rps_values
        results = plot_data.results

    # Generate plots in parallel
    futures = []

    # Submit CDF plots for each (repeat, api, rps)
    for i in range(repeats):
        output_dir = os.path.join(args.output_dir, str(i))
        for api in apis:
            data = results[i][api]
            slo = data[policies[0]][rps_values[0]]["slo"].max() / MS_TO_US
            max_y = slo * 4

            for rps in rps_values:
                futures.append(
                    (
                        _plot_latency_cdf,
                        (
                            output_dir,
                            api,
                            rps,
                            policies,
                            data,
                        ),
                    )
                )

            futures.append(
                (
                    _plot_p99_latency,
                    (
                        output_dir,
                        api,
                        policies,
                        rps_values,
                        data,
                        max_y,
                    ),
                )
            )

    percentiles = [0.80, 0.90, 0.99]
    output_dir = args.output_dir
    for percentile in percentiles:
        for api in apis:
            futures.append(
                (
                    _plot_averaged_percentile_latency,
                    (
                        output_dir,
                        api,
                        percentile,
                        policies,
                        rps_values,
                        results,
                        repeats,
                    ),
                )
            )

    for api in apis:
        futures.append(
            (
                _save_latency_summary_csv,
                (
                    output_dir,
                    api,
                    policies,
                    rps_values,
                    results,
                    repeats,
                ),
            )
        )

    with ThreadPoolExecutor(
        max_workers=get_plot_worker_count(len(futures), max_workers=8)
    ) as executor:
        submitted = [executor.submit(func, *func_args) for func, func_args in futures]
        for future in as_completed(submitted):
            try:
                future.result()
            except Exception as e:
                raise RuntimeError(f"Failed to generate latency plot: {e}") from e


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)
