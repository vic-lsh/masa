import os
from concurrent.futures import ThreadPoolExecutor, as_completed

import matplotlib
import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

matplotlib.use("Agg")  # Use non-interactive backend for thread safety
plt.rcParams["figure.max_open_warning"] = 0

from .util import (
    filter_excluded_errors,
    get_policy_color,
    get_policy_display_name,
    parse_args,
    prepare_output_dir,
    read_data,
)


def _convert_to_milliseconds(df, cols):
    """Convert specified columns from microseconds to milliseconds."""
    MS_TO_US = 10**3
    for col in cols:
        if col in df.columns:
            df[col] /= MS_TO_US


def _get_queueing_columns(df):
    """Return list of columns ending with '_queueing_latency' or known global queueing keys."""
    cols = [c for c in df.columns if c.endswith("_queueing_latency")]
    for k in ["q_lat_init", "q_lat_resume"]:
        if k in df.columns:
            cols.append(k)
    return cols


def _plot_queueing_breakdown(
    output_path: str,
    api: str,
    policies: list,
    rps_values: list,
    data: dict,  # policy -> rps -> df
    title: str,
) -> None:
    """
    Generate breakdown plot for queueing latency by component:
      - Plot: small multiples (one subplot per policy) with stacked bars
      - Stacks: Components (e.g. child1, child2)
    """
    # Identify queueing columns from the first available dataframe
    queueing_cols = []
    for policy in policies:
        if not queueing_cols:
            for rps in rps_values:
                df = data[policy][rps]
                if not df.empty:
                    queueing_cols = _get_queueing_columns(df)
                    if queueing_cols:
                        break

    if not queueing_cols:
        return

    # Sort columns for consistent stacking
    queueing_cols.sort()

    # Clean component names for legend
    # e.g. "child1_queueing_latency" -> "child1"
    def _clean_name(c):
        if c == "q_lat_init":
            return "Ingress"
        if c == "q_lat_resume":
            return "Resume"
        return c.replace("_queueing_latency", "")

    component_names = [_clean_name(c) for c in queueing_cols]

    # Sort policies
    # Reuse sorting logic if possible, or simple sort
    # We'll just use the provided order or simple sort
    sorted_policies = sorted(policies)
    # Use helper if available, but I don't have access to sort_policies_by_type from goodput here unless I duplicate or move it.
    # Let's duplicate the simple sorting logic for now or just sort alphabetically

    rps_values = list(rps_values)
    x = np.arange(len(rps_values))

    # Setup colors for components
    cmap = plt.get_cmap("tab10")
    comp_colors = {comp: cmap(i % cmap.N) for i, comp in enumerate(component_names)}

    n = len(sorted_policies)
    ncols = min(3, max(1, n))
    nrows = int(np.ceil(n / ncols))
    fig, axes = plt.subplots(
        nrows, ncols, figsize=(15, 4 + 2.8 * nrows), sharex=True, sharey=True
    )

    if n == 1:
        axes = [axes]
    elif nrows == 1:
        axes = axes if isinstance(axes, np.ndarray) else [axes]
    else:
        axes = axes.flatten()

    # Determine global max y for consistent scaling
    global_max = 0.0
    for p in sorted_policies:
        for rps in rps_values:
            df = data[p][rps]
            df_filtered = filter_excluded_errors(df)
            if not df_filtered.empty:
                # Average per RPS
                means = df_filtered[queueing_cols].mean()
                total = means.sum()
                global_max = max(global_max, total)

    if global_max <= 0:
        global_max = 1.0
    ymax = global_max * 1.1

    for idx, policy in enumerate(sorted_policies):
        ax = axes[idx]
        ax.grid(axis="y", linestyle="--", alpha=0.4)
        ax.spines["top"].set_visible(False)
        ax.spines["right"].set_visible(False)

        bottom = np.zeros(len(rps_values))

        # Calculate averages for each component per RPS
        comp_values = {c: [] for c in queueing_cols}

        for rps in rps_values:
            df = data[policy][rps]
            df_filtered = filter_excluded_errors(df)
            if df_filtered.empty:
                for c in queueing_cols:
                    comp_values[c].append(0.0)
            else:
                for c in queueing_cols:
                    comp_values[c].append(df_filtered[c].mean())

        for i, col in enumerate(queueing_cols):
            comp_name = component_names[i]
            values = comp_values[col]
            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.78,
                color=comp_colors[comp_name],
                edgecolor="white",
                linewidth=0.4,
                label=comp_name,
            )
            bottom += np.array(values)

        ax.set_title(get_policy_display_name(policy), fontsize=11)
        ax.set_ylim(0, ymax)
        ax.set_xticks(x)
        ax.set_xticklabels([str(v) for v in rps_values], rotation=0)
        ax.set_xlabel("RPS")
        ax.set_ylabel("Avg Queueing Latency (ms)")

    # Hide unused subplots
    for idx in range(n, len(axes)):
        axes[idx].set_visible(False)

    # Shared Legend
    handles = [
        matplotlib.patches.Patch(color=comp_colors[comp], label=comp)
        for comp in component_names
    ]
    fig.legend(
        handles,
        component_names,
        title="Component",
        frameon=False,
        loc="upper center",
        bbox_to_anchor=(0.5, 1.02),
        ncols=min(5, len(component_names)),
    )

    fig.suptitle(title, fontsize=14, y=0.98)
    fig.tight_layout(rect=[0, 0, 1, 0.90])
    fig.savefig(output_path, dpi=300, bbox_inches="tight")
    plt.close(fig)


def _plot_total_queueing_latency(
    output_path: str,
    api: str,
    policies: list,
    rps_values: list,
    data: dict,  # policy -> rps -> df
    title: str,
) -> None:
    """Generate comparison plot for total queueing latency."""
    queueing_cols = []
    # Identify queueing columns
    for policy in policies:
        if not queueing_cols:
            for rps in rps_values:
                df = data[policy][rps]
                if not df.empty:
                    queueing_cols = _get_queueing_columns(df)
                    if queueing_cols:
                        break

    if not queueing_cols:
        return

    fig, ax = plt.subplots(figsize=(12, 6))

    for policy in policies:
        totals = []
        for rps in rps_values:
            df = data[policy][rps]
            df_filtered = filter_excluded_errors(df)
            if df_filtered.empty:
                totals.append(0.0)
            else:
                # Sum queueing cols for each row, then take mean
                # Or mean of each col, then sum. (Equivalent)
                total_avg = df_filtered[queueing_cols].sum(axis=1).mean()
                totals.append(total_avg)

        color = get_policy_color(policy)
        ax.plot(
            rps_values,
            totals,
            marker="o",
            label=get_policy_display_name(policy),
            color=color,
            linewidth=2,
            markersize=6,
        )

    ax.set_ylabel("Avg Total Queueing Latency (ms)")
    ax.set_xlabel("Load (requests per second)")
    ax.set_title(title)
    ax.legend(bbox_to_anchor=(1.05, 1), loc="upper left")
    ax.grid(True, alpha=0.3)

    fig.tight_layout()
    fig.savefig(output_path, dpi=300, bbox_inches="tight")
    plt.close(fig)


def generate_plots(args) -> None:
    prepare_output_dir(args)

    repeats, apis, policies, rps_values, results = read_data(
        args.config_dir, args.data_dir
    )

    MS_TO_US = 10**3

    # Pre-process: convert queueing columns to ms
    for i in range(repeats):
        for api in apis:
            for policy in policies:
                for rps in rps_values:
                    df = results[i][api][policy][rps]
                    q_cols = _get_queueing_columns(df)
                    if q_cols:
                        _convert_to_milliseconds(df, q_cols)

    with ThreadPoolExecutor() as executor:
        futures = []

        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                # Check if this API has queueing data
                has_component_data = False

                for policy in policies:
                    for rps in rps_values:
                        df = results[i][api][policy][rps]
                        if _get_queueing_columns(df):
                            has_component_data = True
                            break
                    if has_component_data:
                        break

                if has_component_data:
                    # Breakdown Plot
                    futures.append(
                        executor.submit(
                            _plot_queueing_breakdown,
                            os.path.join(output_dir, f"queueing_breakdown_{api}.png"),
                            api,
                            policies,
                            rps_values,
                            results[i][api],
                            f"Queueing Latency Breakdown for {api}",
                        )
                    )

                    # Total Plot
                    futures.append(
                        executor.submit(
                            _plot_total_queueing_latency,
                            os.path.join(output_dir, f"queueing_total_{api}.png"),
                            api,
                            policies,
                            rps_values,
                            results[i][api],
                            f"Total Queueing Latency for {api}",
                        )
                    )

        for future in as_completed(futures):
            try:
                future.result()
            except Exception as e:
                # Log but don't fail hard if queueing plots fail, as they are optional/new
                print(f"Warning: Failed to generate queueing plot: {e}")


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)
