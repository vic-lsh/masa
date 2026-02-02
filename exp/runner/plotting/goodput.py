import os
from concurrent.futures import ThreadPoolExecutor, as_completed

import matplotlib
import pandas as pd

matplotlib.use("Agg")  # Use non-interactive backend for thread safety
import matplotlib.pyplot as plt
import numpy as np
from typing import Optional

# Suppress warning about too many open figures when running in parallel
# We properly close all figures, but many may be open simultaneously during parallel execution
plt.rcParams["figure.max_open_warning"] = 0

from .util import (
    parse_args,
    prepare_output_dir,
    read_data,
    filter_excluded_errors,
    get_policy_color,
    get_policy_display_name,
)


def get_request_type_hatch(request_type: str):
    """Get a distinct hatch pattern for each request type.
    Uses consistent hatch patterns to distinguish request types in stacked bars."""
    # Use a consistent set of hatch patterns
    hatches = [
        "",  # no hatch (solid)
        "///",  # diagonal lines
        "\\\\\\",  # back diagonal lines
        "|||",  # vertical lines
        "---",  # horizontal lines
        "+++",  # plus signs
        "xxx",  # cross pattern
        "...",  # dots
        "ooo",  # circles
        "***",  # stars
    ]
    # Use hash of request type to get consistent hatch assignment
    hash_val = hash(request_type) % len(hatches)
    return hatches[hash_val % len(hatches)]


def sort_policies_by_type(policies):
    """Sort policies to group fifo first, then prio_global, then prio_local."""
    fifo_policies = []
    prio_global_policies = []
    prio_local_policies = []
    other_policies = []

    for policy in policies:
        policy_lower = policy.lower()
        if policy_lower.startswith("fifo"):
            fifo_policies.append(policy)
        elif policy_lower.startswith("prio_global"):
            prio_global_policies.append(policy)
        elif policy_lower.startswith("prio_local"):
            prio_local_policies.append(policy)
        else:
            other_policies.append(policy)

    # Sort within each group (base policy before early variant)
    fifo_policies.sort(key=lambda p: (",early" in p.lower(), p))
    prio_global_policies.sort(key=lambda p: (",early" in p.lower(), p))
    prio_local_policies.sort(key=lambda p: (",early" in p.lower(), p))
    other_policies.sort()

    return fifo_policies + prio_global_policies + prio_local_policies + other_policies


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


def compute_goodput_by_request_type(df):
    """Compute goodput broken down by request type (api column).

    Returns:
        dict: Mapping from request type (api) to goodput value
    """
    # Filter out /ClientTimeout and /EarlyReturn errors - they are not goodput
    df_filtered = filter_excluded_errors(df)

    if df_filtered.empty or "api" not in df_filtered.columns:
        return {}

    # Count goodput based on execution time vs SLO
    df_filtered["met_slo"] = df_filtered["latency"] <= df_filtered["slo"]
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    if duration_us == 0:
        return {}

    # Group by api and compute goodput for each
    goodput_by_type = {}
    for api_type in df_filtered["api"].unique():
        api_df = df_filtered[df_filtered["api"] == api_type]
        goodput_by_type[api_type] = api_df["met_slo"].sum() / duration_us * s_to_us

    return goodput_by_type


def compute_early_return_breakdown(df):
    """
    Compute breakdown of early returns by Service:Method.

    Returns:
        dict: Mapping from "Service:Method" -> request rate (req/s)
    """
    if df.empty or "error" not in df.columns:
        return {}

    # Filter for early-return requests only
    early_return_df = df[df["error"].str.startswith("/EarlyReturn")].copy()

    if early_return_df.empty:
        return {}

    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    if duration_us == 0:
        return {}

    # Parse Service and Method from error string
    # Format: /EarlyReturn:<Service>:<Method>
    # If legacy format /EarlyReturn, map to "Unknown:Unknown"
    def parse_error(err):
        if not err.startswith("/EarlyReturn"):
            return "Unknown", "Unknown"
            
        # Strip off the last child part if present (starting with |)
        if "|" in err:
            err = err.split("|")[0]
            
        parts = err.split(":")
        if len(parts) >= 3:
            return parts[1], parts[2]
        return "Unknown", "Unknown"

    early_return_df["parsed"] = early_return_df["error"].apply(parse_error)
    early_return_df["service"] = early_return_df["parsed"].apply(lambda x: x[0])
    early_return_df["method"] = early_return_df["parsed"].apply(lambda x: x[1])
    early_return_df["key"] = (
        early_return_df["service"] + "::" + early_return_df["method"]
    )

    breakdown = {}
    for key in early_return_df["key"].unique():
        count = len(early_return_df[early_return_df["key"] == key])
        breakdown[key] = count / duration_us * s_to_us

    return breakdown


def compute_early_return_last_child_breakdown(df):
    """
    Compute breakdown of early returns by LastChild (Service:Method).

    Returns:
        dict: Mapping from "LastChildService:LastChildMethod" -> request rate (req/s)
    """
    if df.empty or "error" not in df.columns:
        return {}

    # Filter for early-return requests only
    early_return_df = df[df["error"].str.startswith("/EarlyReturn")].copy()

    if early_return_df.empty:
        return {}

    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    if duration_us == 0:
        return {}

    # Parse LastChild from error string
    # Format: /EarlyReturn:<Service>:<Method>|<LastChild>
    def parse_last_child(err):
        if not err.startswith("/EarlyReturn"):
            return "Unknown", "Unknown"
            
        if "|" in err:
            parts = err.split("|", 1)
            if len(parts) < 2:
                return "None", "None"
            last_child = parts[1]
        else:
            # Legacy format with : separator
            parts = err.split(":", 3)
            if len(parts) < 4:
                return "None", "None"
            last_child = parts[3]
        
        if last_child == "None:None":
            return "Ingress", "Drop"
            
        # Try to parse /Service/Method from last_child
        # It usually looks like /package.Service/Method
        child_parts = last_child.split("/")
        if len(child_parts) >= 3:
            return child_parts[-2], child_parts[-1]
            
        return "Unknown", last_child

    early_return_df["parsed"] = early_return_df["error"].apply(parse_last_child)
    early_return_df["service"] = early_return_df["parsed"].apply(lambda x: x[0])
    early_return_df["method"] = early_return_df["parsed"].apply(lambda x: x[1])
    early_return_df["key"] = (
        early_return_df["service"] + "::" + early_return_df["method"]
    )

    breakdown = {}
    for key in early_return_df["key"].unique():
        count = len(early_return_df[early_return_df["key"] == key])
        breakdown[key] = count / duration_us * s_to_us

    return breakdown


def _plot_early_return_breakdown(
    output_path: str,
    *,
    policies: list[str],
    rps_values: list,
    policy_total_early_returns: dict,
    policy_early_returns_breakdown: dict,
    title: str,
) -> None:
    """
    Generate breakdown plot for early-return requests by Service::Method:
      - Plot: small multiples (one subplot per policy) with stacked bars
      - Color: Service
      - Hatch: Method
    """
    sorted_policies = sort_policies_by_type(policies)
    rps_values = list(rps_values)
    x = np.arange(len(rps_values))

    # Collect all unique Services and Methods to assign consistent colors/hatches
    all_services = set()
    all_methods = set()

    for p in sorted_policies:
        per_rps = policy_early_returns_breakdown.get(p, [])
        for d in per_rps:
            for key in (d or {}).keys():
                if "::" in key:
                    svc, mth = key.split("::", 1)
                    all_services.add(svc)
                    all_methods.add(mth)

    sorted_services = sorted(all_services)
    sorted_methods = sorted(all_methods)

    # Color mapping for services
    svc_cmap = plt.get_cmap("tab10" if len(sorted_services) <= 10 else "tab20")
    service_colors = {
        svc: svc_cmap(i % svc_cmap.N) for i, svc in enumerate(sorted_services)
    }

    # Hatch mapping for methods
    hatches = ["", "///", "\\\\", "|||", "---", "+++", "xxx", "ooo", "...", "***"]
    method_hatches = {
        mth: hatches[i % len(hatches)] for i, mth in enumerate(sorted_methods)
    }

    # Generate output path
    breakdown_path = output_path.replace(".png", "_breakdown.png")

    # ===== Plot: Breakdown by Service::Method =====
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

    # Global y-limit
    global_max = 0.0
    for p in sorted_policies:
        vals = policy_total_early_returns.get(p, [])
        if vals:
            global_max = max(global_max, max(float(v or 0.0) for v in vals))
    if global_max <= 0:
        global_max = 1.0
    ymax = global_max * 1.08

    for idx, policy in enumerate(sorted_policies):
        ax = axes[idx]
        _style_axes(ax)

        bottom = np.zeros(len(rps_values))
        per_rps = policy_early_returns_breakdown.get(policy, [])

        # Iterate over all possible (Service, Method) pairs to maintain stack order?
        # Or just iterate over present keys?
        # To be safe and consistent, let's iterate over sorted keys present in this policy
        # Actually, let's sort by Service then Method
        present_keys = set()
        for d in per_rps:
            present_keys.update((d or {}).keys())

        sorted_keys = sorted(present_keys)

        for key in sorted_keys:
            svc, mth = key.split("::", 1)
            values = []
            for i in range(len(rps_values)):
                if i < len(per_rps) and per_rps[i] is not None:
                    values.append(float(per_rps[i].get(key, 0.0) or 0.0))
                else:
                    values.append(0.0)

            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.78,
                color=service_colors.get(svc, "grey"),
                hatch=method_hatches.get(mth, ""),
                edgecolor="white",
                linewidth=0.4,
                label=key,  # Label will be used for legend later if we wanted per-item legend
            )
            bottom += np.array(values)

        ax.set_title(get_policy_display_name(policy), fontsize=11)
        ax.set_ylim(0, ymax)
        ax.set_xticks(x)
        ax.set_xticklabels([str(v) for v in rps_values], rotation=0)
        ax.set_xlabel("RPS")
        ax.set_ylabel("Early-return rate (req/s)")

    # Hide unused subplots
    for idx in range(n, len(axes)):
        axes[idx].set_visible(False)

    # Double Legend: One for Services (Colors), One for Methods (Hatches)
    # Create proxy artists
    service_handles = [
        matplotlib.patches.Patch(
            facecolor=service_colors[svc], label=svc, edgecolor="black", linewidth=0.5
        )
        for svc in sorted_services
    ]
    method_handles = [
        matplotlib.patches.Patch(
            facecolor="white",
            hatch=method_hatches[mth],
            label=mth,
            edgecolor="black",
            linewidth=0.5,
        )
        for mth in sorted_methods
    ]

    # Legend 1: Services
    if service_handles:
        legend1 = fig.legend(
            service_handles,
            sorted_services,
            title="Service (Color)",
            frameon=False,
            loc="upper center",
            bbox_to_anchor=(0.3, 1.02),
            ncols=min(4, len(sorted_services)),
        )
        fig.add_artist(legend1)

    # Legend 2: Methods
    if method_handles:
        fig.legend(
            method_handles,
            sorted_methods,
            title="Method (Hatch)",
            frameon=False,
            loc="upper center",
            bbox_to_anchor=(0.7, 1.02),
            ncols=min(4, len(sorted_methods)),
        )

    fig.suptitle(
        title, fontsize=14, y=1.05
    )  # Moved up slightly to make room for legends
    fig.tight_layout(rect=[0, 0, 1, 0.90])
    fig.savefig(breakdown_path, dpi=300, bbox_inches="tight")
    plt.close(fig)

    # Save CSV
    csv_path = output_path.replace(".png", "_breakdown.csv")
    csv_data = []
    for policy in sorted_policies:
        per_rps = policy_early_returns_breakdown.get(policy, [])
        for i, rps in enumerate(rps_values):
            if i < len(per_rps) and per_rps[i] is not None:
                for key, val in per_rps[i].items():
                    svc, mth = key.split("::", 1)
                    csv_data.append({
                        "RPS": rps,
                        "Policy": policy,
                        "Service": svc,
                        "Method": mth,
                        "Rate": float(val or 0.0)
                    })
    
    if csv_data:
        pd.DataFrame(csv_data).to_csv(csv_path, index=False)


def compute_slo_miss_by_request_type(df):
    """Compute SLO miss rate broken down by request type (api column).

    Returns:
        dict: Mapping from request type (api) to SLO miss rate (requests per second)
    """
    # Filter out /ClientTimeout and /EarlyReturn errors - they are not "SLO misses"
    df_filtered = filter_excluded_errors(df)

    if (
        df_filtered.empty
        or "api" not in df_filtered.columns
        or "slo" not in df_filtered.columns
    ):
        return {}

    # Identify misses (latency > slo)
    miss_df = df_filtered[df_filtered["latency"] > df_filtered["slo"]]

    if miss_df.empty:
        return {}

    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    if duration_us == 0:
        return {}

    miss_by_type = {}
    for api_type in miss_df["api"].unique():
        api_df = miss_df[miss_df["api"] == api_type]
        miss_by_type[api_type] = len(api_df) / duration_us * s_to_us

    return miss_by_type


def _style_axes(ax):
    ax.grid(axis="y", linestyle="--", alpha=0.4)
    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)


def _request_type_order_and_collapse(
    policy_goodputs_by_type: dict,
    *,
    max_types: int = 7,
):
    """
    Decide a stable ordering of request types and optionally collapse the tail
    into an "Other" bucket to reduce visual clutter.

    Args:
        policy_goodputs_by_type: policy -> list[dict[request_type, goodput]] (one per RPS)
        max_types: Maximum number of distinct stacks to show (including "Other" if used).
    """
    totals = {}
    for _policy, per_rps in (policy_goodputs_by_type or {}).items():
        for d in per_rps or []:
            for rt, v in (d or {}).items():
                totals[rt] = totals.get(rt, 0.0) + float(v or 0.0)

    request_types = sorted(totals.keys(), key=lambda rt: (-totals[rt], str(rt)))
    if max_types is not None and max_types > 0 and len(request_types) > max_types:
        keep = request_types[: max_types - 1]
        collapsed = request_types[max_types - 1 :]
        return keep, collapsed
    return request_types, []


def _collapse_request_types_for_policy(
    per_rps_list: list,
    *,
    keep: list,
    collapsed: list,
    other_label: str = "Other",
):
    """Return a new per-rps list with request types collapsed into Other."""
    if not collapsed:
        return per_rps_list
    collapsed_set = set(collapsed)
    out = []
    for d in per_rps_list:
        d = d or {}
        new_d = {rt: float(d.get(rt, 0.0) or 0.0) for rt in keep}
        other = 0.0
        for rt, v in d.items():
            if rt in collapsed_set:
                other += float(v or 0.0)
        if other > 0:
            new_d[other_label] = other
        out.append(new_d)
    return out


def _get_request_type_colors_mapping(all_request_types: list[str]) -> dict:
    """Create a stable color mapping for all request types.

    This ensures that each API gets the same color across all plots.
    Args:
        all_request_types: All possible request types (sorted for consistency)

    Returns:
        dict: Mapping from request type to color
    """
    # Good defaults for 2..10 types; falls back cleanly for larger.
    cmap = plt.get_cmap("tab10" if len(all_request_types) <= 10 else "tab20")
    return {rt: cmap(i % cmap.N) for i, rt in enumerate(all_request_types)}


def _get_request_type_colors(
    request_types: list[str], color_mapping: Optional[dict] = None
):
    """Get colors for a subset of request types.

    If color_mapping is provided, uses it for consistent colors across plots.
    Otherwise, creates colors on-the-fly (legacy behavior).
    """
    if color_mapping is not None:
        return {rt: color_mapping.get(rt, "grey") for rt in request_types}
    # Legacy behavior: create colors on-the-fly
    cmap = plt.get_cmap("tab10" if len(request_types) <= 10 else "tab20")
    return {rt: cmap(i % cmap.N) for i, rt in enumerate(request_types)}


def _plot_slo_miss_breakdown(
    output_path: str,
    *,
    policies: list[str],
    rps_values: list,
    policy_total_slo_misses: dict,
    policy_slo_misses_by_type: dict,
    title: str,
    subtitle: Optional[str] = None,
    request_type_color_mapping: Optional[dict] = None,
) -> None:
    """
    Generate breakdown plot for SLO-miss requests by API:
      - Plot: small multiples (one subplot per policy) with stacked bars for API breakdown
    """
    sorted_policies = sort_policies_by_type(policies)
    rps_values = list(rps_values)
    x = np.arange(len(rps_values))

    # Decide request types + optional collapse
    keep, collapsed = _request_type_order_and_collapse(
        policy_slo_misses_by_type, max_types=7
    )
    request_types = list(keep)
    if collapsed:
        request_types.append("Other")
    rt_colors = _get_request_type_colors(request_types, request_type_color_mapping)

    # Collapse tail per policy if needed
    policy_slo_misses_by_type_collapsed = {}
    for p in sorted_policies:
        per_rps = (policy_slo_misses_by_type or {}).get(p, [])
        policy_slo_misses_by_type_collapsed[p] = _collapse_request_types_for_policy(
            per_rps,
            keep=keep,
            collapsed=collapsed,
            other_label="Other",
        )

    # Generate output path for breakdown plot
    breakdown_path = output_path.replace(".png", "_breakdown.png")

    # ===== Plot: Breakdown by API with shared axes =====
    n = len(sorted_policies)
    ncols = min(3, max(1, n))
    nrows = int(np.ceil(n / ncols))
    fig, axes = plt.subplots(
        nrows, ncols, figsize=(15, 4 + 2.8 * nrows), sharex=True, sharey=True
    )

    # Handle case where there's only one subplot
    if n == 1:
        axes = [axes]
    elif nrows == 1:
        axes = axes if isinstance(axes, np.ndarray) else [axes]
    else:
        axes = axes.flatten()

    # Global y-limit for comparability across subplots
    global_max = 0.0
    for p in sorted_policies:
        vals = policy_total_slo_misses.get(p, [])
        if vals:
            global_max = max(global_max, max(float(v or 0.0) for v in vals))
    if global_max <= 0:
        global_max = 1.0
    ymax = global_max * 1.08

    for idx, policy in enumerate(sorted_policies):
        ax = axes[idx]
        _style_axes(ax)

        bottom = np.zeros(len(rps_values))
        per_rps = policy_slo_misses_by_type_collapsed.get(policy, [])
        for rt in request_types:
            values = []
            for i in range(len(rps_values)):
                if i < len(per_rps) and per_rps[i] is not None:
                    values.append(float(per_rps[i].get(rt, 0.0) or 0.0))
                else:
                    values.append(0.0)
            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.78,
                color=rt_colors[rt],
                edgecolor="white",
                linewidth=0.4,
                label=rt,
            )
            bottom += np.array(values)

        ax.set_title(get_policy_display_name(policy), fontsize=11)
        ax.set_ylim(0, ymax)
        ax.set_xticks(x)
        ax.set_xticklabels([str(v) for v in rps_values], rotation=0)
        ax.set_xlabel("RPS")
        ax.set_ylabel("SLO miss rate (req/s)")

    # Hide unused subplots
    for idx in range(n, len(axes)):
        axes[idx].set_visible(False)

    # One shared legend for request types
    handles, labels = [], []
    if n > 0:
        # Build stable handle/label list from colors
        for rt in request_types:
            patch = matplotlib.patches.Patch(color=rt_colors[rt], label=rt)
            handles.append(patch)
            labels.append(rt)
    fig.legend(
        handles,
        labels,
        title="API",
        frameon=False,
        loc="upper center",
        bbox_to_anchor=(0.5, 1.02),
        ncols=len(request_types),
    )

    fig.suptitle(title, fontsize=14, y=0.98)
    fig.tight_layout(rect=[0, 0, 1, 0.90])
    fig.savefig(breakdown_path, dpi=300, bbox_inches="tight")
    plt.close(fig)


def _plot_all_api_goodput_clean(
    output_path: str,
    *,
    policies: list[str],
    rps_values: list,
    policy_total_goodputs: dict,
    policy_goodputs_by_type: dict,
    title: str,
    subtitle: Optional[str] = None,
    request_type_color_mapping: Optional[dict] = None,
) -> None:
    """
    Generate two separate plots for ALL:
      - Plot 1: aggregated goodput vs load (bar chart) for policy comparison
      - Plot 2: small multiples (one subplot per policy) with stacked bars for request-type breakdown
    """
    sorted_policies = sort_policies_by_type(policies)
    rps_values = list(rps_values)
    x = np.arange(len(rps_values))

    # Decide request types + optional collapse
    keep, collapsed = _request_type_order_and_collapse(
        policy_goodputs_by_type, max_types=7
    )
    request_types = list(keep)
    if collapsed:
        request_types.append("Other")
    rt_colors = _get_request_type_colors(request_types, request_type_color_mapping)

    # Collapse tail per policy if needed
    policy_goodputs_by_type_collapsed = {}
    for p in sorted_policies:
        per_rps = (policy_goodputs_by_type or {}).get(p, [])
        policy_goodputs_by_type_collapsed[p] = _collapse_request_types_for_policy(
            per_rps,
            keep=keep,
            collapsed=collapsed,
            other_label="Other",
        )

    # Generate output paths for three separate plots
    base_path = output_path.replace(".png", "")
    aggregated_path = f"{base_path}_aggregated.png"
    breakdown_path = f"{base_path}_breakdown.png"
    fraction_path = f"{base_path}_fraction.png"

    # ===== Plot 1: Aggregated goodput only (line graph) =====
    fig1 = plt.figure(figsize=(12, 6))
    ax1 = fig1.add_subplot(1, 1, 1)
    _style_axes(ax1)

    for policy in sorted_policies:
        y = policy_total_goodputs.get(policy, [])
        color = get_policy_color(policy)
        ax1.plot(
            rps_values,
            y,
            marker="o",
            label=get_policy_display_name(policy),
            color=color,
            linewidth=2,
            markersize=6,
        )
    ax1.set_ylabel("Goodput (req/s meeting SLO)")
    ax1.set_xlabel("Load (requests per second)")
    ax1.set_title(
        title
        if "aggregated" in title.lower() or "total" in title.lower()
        else f"{title} - Aggregated"
    )
    ax1.legend(ncols=3, frameon=False, loc="upper left")

    fig1.tight_layout()
    fig1.savefig(aggregated_path, dpi=300, bbox_inches="tight")
    plt.close(fig1)

    # ===== Plot 3: Goodput as fraction of offered load (line graph) =====
    fig3 = plt.figure(figsize=(12, 6))
    ax3 = fig3.add_subplot(1, 1, 1)
    _style_axes(ax3)

    for policy in sorted_policies:
        goodput_values = policy_total_goodputs.get(policy, [])
        # Calculate goodput fraction: goodput / offered load (RPS)
        fraction_values = []
        for i, rps in enumerate(rps_values):
            if i < len(goodput_values) and rps > 0:
                fraction_values.append(goodput_values[i] / rps)
            else:
                fraction_values.append(0.0)
        color = get_policy_color(policy)
        ax3.plot(
            rps_values,
            fraction_values,
            marker="o",
            label=get_policy_display_name(policy),
            color=color,
            linewidth=2,
            markersize=6,
        )
    ax3.set_ylabel("Goodput / Offered Load")
    ax3.set_xlabel("Load (requests per second)")
    ax3.set_title(
        title
        if "aggregated" in title.lower() or "total" in title.lower()
        else f"{title} - Goodput Fraction"
    )
    ax3.legend(ncols=3, frameon=False, loc="upper left")
    ax3.set_ylim(0, 1.1)  # Goodput fraction should be between 0 and 1

    fig3.tight_layout()
    fig3.savefig(fraction_path, dpi=300, bbox_inches="tight")
    plt.close(fig3)

    # ===== Plot 2: Breakdown by request type with shared axes =====
    n = len(sorted_policies)
    ncols = min(3, max(1, n))
    nrows = int(np.ceil(n / ncols))
    fig2, axes = plt.subplots(
        nrows, ncols, figsize=(15, 4 + 2.8 * nrows), sharex=True, sharey=True
    )

    # Handle case where there's only one subplot
    if n == 1:
        axes = [axes]
    elif nrows == 1:
        axes = axes if isinstance(axes, np.ndarray) else [axes]
    else:
        axes = axes.flatten()

    # Global y-limit for comparability across subplots
    global_max = 0.0
    for p in sorted_policies:
        vals = policy_total_goodputs.get(p, [])
        if vals:
            global_max = max(global_max, max(float(v or 0.0) for v in vals))
    if global_max <= 0:
        global_max = 1.0
    ymax = global_max * 1.08

    for idx, policy in enumerate(sorted_policies):
        ax = axes[idx]
        _style_axes(ax)

        bottom = np.zeros(len(rps_values))
        per_rps = policy_goodputs_by_type_collapsed.get(policy, [])
        for rt in request_types:
            values = []
            for i in range(len(rps_values)):
                if i < len(per_rps) and per_rps[i] is not None:
                    values.append(float(per_rps[i].get(rt, 0.0) or 0.0))
                else:
                    values.append(0.0)
            ax.bar(
                x,
                values,
                bottom=bottom,
                width=0.78,
                color=rt_colors[rt],
                edgecolor="white",
                linewidth=0.4,
                label=rt,
            )
            bottom += np.array(values)

        ax.set_title(get_policy_display_name(policy), fontsize=11)
        ax.set_ylim(0, ymax)
        ax.set_xticks(x)
        ax.set_xticklabels([str(v) for v in rps_values], rotation=0)
        ax.set_xlabel("RPS")
        ax.set_ylabel("Goodput")

    # Hide unused subplots
    for idx in range(n, len(axes)):
        axes[idx].set_visible(False)

    # One shared legend for request types
    handles, labels = [], []
    if n > 0:
        # Build stable handle/label list from colors
        for rt in request_types:
            patch = matplotlib.patches.Patch(color=rt_colors[rt], label=rt)
            handles.append(patch)
            labels.append(rt)
    fig2.legend(
        handles,
        labels,
        title="Request type",
        frameon=False,
        loc="upper center",
        bbox_to_anchor=(0.5, 1.02),
        ncols=len(request_types),
    )

    fig2.tight_layout(rect=[0, 0, 1, 0.90])
    fig2.savefig(breakdown_path, dpi=300, bbox_inches="tight")
    plt.close(fig2)

    # Save Aggregated Goodput CSV
    agg_csv_path = base_path + "_aggregated.csv"
    agg_data = []
    for policy in sorted_policies:
        goodput_values = policy_total_goodputs.get(policy, [])
        for i, rps in enumerate(rps_values):
            if i < len(goodput_values):
                gp = float(goodput_values[i] or 0.0)
                agg_data.append({
                    "RPS": rps,
                    "Policy": policy,
                    "Goodput": gp,
                    "Fraction": gp / rps if rps > 0 else 0.0
                })
    if agg_data:
        pd.DataFrame(agg_data).to_csv(agg_csv_path, index=False)

    # Save Breakdown Goodput CSV
    breakdown_csv_path = base_path + "_breakdown.csv"
    breakdown_data = []
    for policy in sorted_policies:
        per_rps = policy_goodputs_by_type.get(policy, [])
        for i, rps in enumerate(rps_values):
            if i < len(per_rps) and per_rps[i] is not None:
                for rt, val in per_rps[i].items():
                    breakdown_data.append({
                        "RPS": rps,
                        "Policy": policy,
                        "RequestType": rt,
                        "Goodput": float(val or 0.0)
                    })
    if breakdown_data:
        pd.DataFrame(breakdown_data).to_csv(breakdown_csv_path, index=False)


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
        completion_times = (
            df_filtered.loc[met_slo_mask, "start_at"]
            + df_filtered.loc[met_slo_mask, "latency"]
        )
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
    policy_goodputs_by_type: Optional[dict] = None,
    request_type_color_mapping: Optional[dict] = None,
) -> None:
    """Generate policy goodput comparison plot for a specific repeat and API.

    Args:
        policy_goodputs_by_type: Optional dict mapping policy -> list of dicts (one per RPS)
            where each dict maps request_type -> goodput. Used for stacked bars when api == "ALL".
        request_type_color_mapping: Optional dict mapping request type to color for consistency.
    """
    if api == "ALL" and policy_goodputs_by_type is not None:
        output_path = os.path.join(output_dir, f"goodput_{api}.png")
        _plot_all_api_goodput_clean(
            output_path,
            policies=policies,
            rps_values=rps_values,
            policy_total_goodputs=policy_goodputs,
            policy_goodputs_by_type=policy_goodputs_by_type,
            title="Goodput vs load (ALL) and breakdown by request type",
            subtitle="Panel A: total goodput; Panel B: stacked bars per policy",
            request_type_color_mapping=request_type_color_mapping,
        )
        return

    fig, ax = plt.subplots(figsize=(12, 6))
    sorted_policies = sort_policies_by_type(policies)
    bar_width = 0.12
    index = np.arange(len(rps_values))

    for j, policy in enumerate(sorted_policies):
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        color = get_policy_color(policy)
        ax.bar(
            index + offset,
            policy_goodputs[policy],
            bar_width,
            label=get_policy_display_name(policy),
            color=color,
        )

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("Goodput (requests meeting SLO per second)")
    ax.set_title(f"Goodput Comparison by Policy and RPS for {api} API")
    ax.set_xticks(index)
    ax.set_xticklabels([str(rps) for rps in rps_values])
    ax.legend(bbox_to_anchor=(1.05, 1), loc="upper left")
    _style_axes(ax)
    fig.tight_layout()
    fig.savefig(
        os.path.join(output_dir, f"goodput_{api}.png"),
        dpi=300,
        bbox_inches="tight",
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
    policy_goodputs_by_type: Optional[list] = None,
    request_type_color_mapping: Optional[dict] = None,
) -> None:
    """Generate averaged goodput comparison plot for a specific API.

    Args:
        policy_goodputs_by_type: Optional list (one per repeat) of dicts mapping
            policy -> list of dicts (one per RPS) where each dict maps request_type -> goodput.
            Used for stacked bars when api == "ALL".
        request_type_color_mapping: Optional dict mapping request type to color for consistency.
    """
    sorted_policies = sort_policies_by_type(policies)
    index = np.arange(len(rps_values))

    if api == "ALL" and policy_goodputs_by_type is not None:
        # Build averaged totals and averaged breakdown dict in the same shape as per-repeat plotter expects.
        avg_totals = {}
        for policy in sorted_policies:
            avg_totals[policy] = (
                sum(np.array(policy_goodputs[i][api][policy]) for i in range(repeats))
                / repeats
            ).tolist()

        # Average breakdown per (policy, rps, request_type)
        avg_breakdown = {
            policy: [dict() for _ in range(len(rps_values))]
            for policy in sorted_policies
        }
        # Collect all request types present anywhere
        all_types = set()
        for i in range(repeats):
            if i < len(policy_goodputs_by_type):
                for policy in policies:
                    per_rps = policy_goodputs_by_type[i].get(policy, [])
                    for d in per_rps:
                        all_types.update((d or {}).keys())
        for policy in sorted_policies:
            for rps_idx in range(len(rps_values)):
                for rt in all_types:
                    vals = []
                    for i in range(repeats):
                        if i >= len(policy_goodputs_by_type):
                            continue
                        per_rps = policy_goodputs_by_type[i].get(policy, [])
                        if (
                            rps_idx < len(per_rps)
                            and per_rps[rps_idx] is not None
                            and rt in per_rps[rps_idx]
                        ):
                            vals.append(float(per_rps[rps_idx][rt]))
                    if vals:
                        avg_breakdown[policy][rps_idx][rt] = sum(vals) / len(vals)

        output_path = os.path.join(output_dir, f"goodput_{api}.png")
        _plot_all_api_goodput_clean(
            output_path,
            policies=policies,
            rps_values=rps_values,
            policy_total_goodputs=avg_totals,
            policy_goodputs_by_type=avg_breakdown,
            title=f"Average goodput vs load (ALL) and breakdown by request type",
            subtitle=f"Averaged over {repeats} run(s). Panel A: total; Panel B: per-policy stacked bars.",
            request_type_color_mapping=request_type_color_mapping,
        )
        return

    fig, ax = plt.subplots(figsize=(12, 6))
    bar_width = 0.12

    for j, policy in enumerate(sorted_policies):
        average_goodput = (
            sum(np.array(policy_goodputs[i][api][policy]) for i in range(repeats))
            / repeats
        )
        offset = (j - len(sorted_policies) / 2 + 0.5) * bar_width
        color = get_policy_color(policy)
        ax.bar(
            index + offset,
            average_goodput,
            bar_width,
            label=get_policy_display_name(policy),
            color=color,
        )

    ax.set_xlabel("Requests Per Second (RPS)")
    ax.set_ylabel("average goodput (requests meeting SLO per second)")
    ax.set_title(
        f"average goodput comparison by policy and RPS for {api} API over {repeats} runs"
    )
    ax.set_xticks(index)
    ax.set_xticklabels([str(rps) for rps in rps_values])
    ax.legend(bbox_to_anchor=(1.05, 1), loc="upper left")
    _style_axes(ax)
    fig.tight_layout()
    fig.savefig(
        os.path.join(output_dir, f"goodput_{api}.png"),
        dpi=300,
        bbox_inches="tight",
    )
    plt.close(fig)


def generate_plots(args) -> None:
    prepare_output_dir(args)

    repeats, apis, policies, rps_values, results = read_data(
        args.config_dir, args.data_dir
    )

    # First, compute all policy goodputs (needed for plots)
    policy_goodputs = []
    # For "ALL" API, also compute goodput broken down by request type
    policy_goodputs_by_type = []
    # For "ALL" API, compute early-return breakdown by request type
    policy_early_returns_by_type = []
    policy_total_early_returns = []
    # For "ALL" API, compute early-return breakdown by LAST CHILD
    policy_early_returns_last_child_by_type = []
    policy_total_early_returns_last_child = []
    # For "ALL" API, compute SLO miss breakdown by request type
    policy_slo_misses_by_type = []
    policy_total_slo_misses = []

    # Collect all unique request types for consistent coloring across all plots
    all_request_types = set()

    for i in range(repeats):
        policy_goodputs.append({})
        policy_goodputs_by_type.append({})
        policy_early_returns_by_type.append({})
        policy_total_early_returns.append({})
        policy_slo_misses_by_type.append({})
        policy_total_slo_misses.append({})
        for api in apis:
            data = results[i][api]
            policy_goodputs[i][api] = {
                policy: [compute_goodput(data[policy][rps]) for rps in rps_values]
                for policy in policies
            }
            # For "ALL" API, compute breakdown by request type
            if api == "ALL":
                policy_goodputs_by_type[i][api] = {
                    policy: [
                        compute_goodput_by_request_type(data[policy][rps])
                        for rps in rps_values
                    ]
                    for policy in policies
                }
                # Collect all request types for consistent coloring
                for policy in policies:
                    for rps_dict in policy_goodputs_by_type[i][api][policy]:
                        all_request_types.update(rps_dict.keys())

                # Compute early-return breakdown by request type
                policy_early_returns_by_type[i][api] = {
                    policy: [
                        compute_early_return_breakdown(data[policy][rps])
                        for rps in rps_values
                    ]
                    for policy in policies
                }
                # Collect request types from early returns as well (these are now Service::Method keys)
                for policy in policies:
                    for rps_dict in policy_early_returns_by_type[i][api][policy]:
                        all_request_types.update(rps_dict.keys())

                # Compute total early-return rate per policy
                policy_total_early_returns[i][api] = {
                    policy: [
                        sum(d.values())
                        for d in policy_early_returns_by_type[i][api][policy]
                    ]
                    for policy in policies
                }
                
                # Compute early-return breakdown by LAST CHILD
                policy_early_returns_last_child_by_type.append({})
                policy_total_early_returns_last_child.append({})
                policy_early_returns_last_child_by_type[i][api] = {
                    policy: [
                        compute_early_return_last_child_breakdown(data[policy][rps])
                        for rps in rps_values
                    ]
                    for policy in policies
                }
                # Collect keys
                for policy in policies:
                    for rps_dict in policy_early_returns_last_child_by_type[i][api][policy]:
                        all_request_types.update(rps_dict.keys())
                        
                # Compute total
                policy_total_early_returns_last_child[i][api] = {
                    policy: [
                        sum(d.values())
                        for d in policy_early_returns_last_child_by_type[i][api][policy]
                    ]
                    for policy in policies
                }

                # Compute SLO miss breakdown by request type

                policy_slo_misses_by_type[i][api] = {
                    policy: [
                        compute_slo_miss_by_request_type(data[policy][rps])
                        for rps in rps_values
                    ]
                    for policy in policies
                }
                # Collect request types from SLO misses as well
                for policy in policies:
                    for rps_dict in policy_slo_misses_by_type[i][api][policy]:
                        all_request_types.update(rps_dict.keys())

                # Compute total SLO miss rate per policy
                policy_total_slo_misses[i][api] = {
                    policy: [
                        sum(
                            compute_slo_miss_by_request_type(data[policy][rps]).values()
                        )
                        for rps in rps_values
                    ]
                    for policy in policies
                }

    # Create stable color mapping for all request types
    request_type_color_mapping = {}
    if all_request_types:
        sorted_request_types = sorted(all_request_types)
        request_type_color_mapping = _get_request_type_colors_mapping(
            sorted_request_types
        )

    # Generate plots in parallel
    with ThreadPoolExecutor() as executor:
        futures = []

        # Submit policy goodput comparison plots for each (repeat, api)
        # Only generate plots for "ALL" API, skip individual APIs
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                if api == "ALL":
                    goodputs_by_type = None
                    if i < len(policy_goodputs_by_type):
                        goodputs_by_type = policy_goodputs_by_type[i].get(api)
                    futures.append(
                        executor.submit(
                            _plot_policy_goodput_comparison,
                            output_dir,
                            api,
                            policies,
                            rps_values,
                            policy_goodputs[i][api],
                            goodputs_by_type,
                            request_type_color_mapping,
                        )
                    )

        # Submit averaged goodput plots for each api
        # Only generate plots for "ALL" API, skip individual APIs
        output_dir = args.output_dir
        for api in apis:
            if api == "ALL":
                goodputs_by_type = None
                # Extract the "ALL" data from each repeat
                goodputs_by_type = [
                    policy_goodputs_by_type[i].get("ALL", {}) for i in range(repeats)
                ]
                futures.append(
                    executor.submit(
                        _plot_averaged_goodput,
                        output_dir,
                        api,
                        policies,
                        rps_values,
                        policy_goodputs,
                        repeats,
                        goodputs_by_type,
                        request_type_color_mapping,
                    )
                )

        # Submit early-return breakdown plots for each repeat
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                if api == "ALL":
                    early_returns_by_type = policy_early_returns_by_type[i].get(api)
                    total_early_returns = policy_total_early_returns[i].get(api)
                    if (
                        early_returns_by_type is not None
                        and total_early_returns is not None
                    ):
                        output_path = os.path.join(
                            output_dir, f"early_return_{api}.png"
                        )
                        futures.append(
                            executor.submit(
                                _plot_early_return_breakdown,
                                output_path,
                                policies=policies,
                                rps_values=rps_values,
                                policy_total_early_returns=total_early_returns,
                                policy_early_returns_breakdown=early_returns_by_type,
                                title="Early-return requests breakdown by Service::Method",
                            )
                        )

                    early_returns_last_child_by_type = policy_early_returns_last_child_by_type[i].get(api)
                    total_early_returns_last_child = policy_total_early_returns_last_child[i].get(api)
                    if (
                        early_returns_last_child_by_type is not None
                        and total_early_returns_last_child is not None
                    ):
                        output_path = os.path.join(
                            output_dir, f"early_return_last_child_{api}.png"
                        )
                        futures.append(
                            executor.submit(
                                _plot_early_return_breakdown,
                                output_path,
                                policies=policies,
                                rps_values=rps_values,
                                policy_total_early_returns=total_early_returns_last_child,
                                policy_early_returns_breakdown=early_returns_last_child_by_type,
                                title="Early-return requests breakdown by Last Child Service::Method",
                            )
                        )

        # Submit SLO miss breakdown plots for each repeat
        for i in range(repeats):
            output_dir = os.path.join(args.output_dir, str(i))
            for api in apis:
                if api == "ALL":
                    slo_misses_by_type = policy_slo_misses_by_type[i].get(api)
                    total_slo_misses = policy_total_slo_misses[i].get(api)
                    if slo_misses_by_type is not None and total_slo_misses is not None:
                        output_path = os.path.join(output_dir, f"slo_miss_{api}.png")
                        futures.append(
                            executor.submit(
                                _plot_slo_miss_breakdown,
                                output_path,
                                policies=policies,
                                rps_values=rps_values,
                                policy_total_slo_misses=total_slo_misses,
                                policy_slo_misses_by_type=slo_misses_by_type,
                                title="SLO-miss requests breakdown by API",
                                request_type_color_mapping=request_type_color_mapping,
                            )
                        )

        # Submit averaged early-return breakdown plot
        output_dir = args.output_dir
        for api in apis:
            if api == "ALL":
                # Average breakdown per (policy, rps, request_type)
                avg_total_early_returns = {}
                avg_breakdown = {
                    policy: [dict() for _ in range(len(rps_values))]
                    for policy in policies
                }

                # Collect all request types present anywhere
                all_types = set()
                for i in range(repeats):
                    if i < len(policy_early_returns_by_type):
                        for policy in policies:
                            per_rps = (
                                policy_early_returns_by_type[i]
                                .get(api, {})
                                .get(policy, [])
                            )
                            for d in per_rps:
                                all_types.update((d or {}).keys())

                # Average totals
                for policy in policies:
                    totals = []
                    for rps_idx in range(len(rps_values)):
                        vals = []
                        for i in range(repeats):
                            if i < len(policy_total_early_returns):
                                per_rps = (
                                    policy_total_early_returns[i]
                                    .get(api, {})
                                    .get(policy, [])
                                )
                                if rps_idx < len(per_rps):
                                    vals.append(float(per_rps[rps_idx] or 0.0))
                        if vals:
                            totals.append(sum(vals) / len(vals))
                        else:
                            totals.append(0.0)
                    avg_total_early_returns[policy] = totals

                # Average breakdown
                for policy in policies:
                    for rps_idx in range(len(rps_values)):
                        for rt in all_types:
                            vals = []
                            for i in range(repeats):
                                if i >= len(policy_early_returns_by_type):
                                    continue
                                per_rps = (
                                    policy_early_returns_by_type[i]
                                    .get(api, {})
                                    .get(policy, [])
                                )
                                if (
                                    rps_idx < len(per_rps)
                                    and per_rps[rps_idx] is not None
                                    and rt in per_rps[rps_idx]
                                ):
                                    vals.append(float(per_rps[rps_idx][rt]))
                            if vals:
                                avg_breakdown[policy][rps_idx][rt] = sum(vals) / len(
                                    vals
                                )

                output_path = os.path.join(output_dir, f"early_return_{api}.png")
                futures.append(
                    executor.submit(
                        _plot_early_return_breakdown,
                        output_path,
                        policies=policies,
                        rps_values=rps_values,
                        policy_total_early_returns=avg_total_early_returns,
                        policy_early_returns_breakdown=avg_breakdown,
                        title=f"Average early-return requests breakdown by Service::Method (averaged over {repeats} run(s))",
                    )
                )

                # Average breakdown for LAST CHILD
                avg_total_early_returns_lc = {}
                avg_breakdown_lc = {
                    policy: [dict() for _ in range(len(rps_values))]
                    for policy in policies
                }
                
                all_types_lc = set()
                for i in range(repeats):
                    if i < len(policy_early_returns_last_child_by_type):
                        for policy in policies:
                            per_rps = (
                                policy_early_returns_last_child_by_type[i]
                                .get(api, {})
                                .get(policy, [])
                            )
                            for d in per_rps:
                                all_types_lc.update((d or {}).keys())

                for policy in policies:
                    totals = []
                    for rps_idx in range(len(rps_values)):
                        vals = []
                        for i in range(repeats):
                            if i < len(policy_total_early_returns_last_child):
                                per_rps = (
                                    policy_total_early_returns_last_child[i]
                                    .get(api, {})
                                    .get(policy, [])
                                )
                                if rps_idx < len(per_rps):
                                    vals.append(float(per_rps[rps_idx] or 0.0))
                        if vals:
                            totals.append(sum(vals) / len(vals))
                        else:
                            totals.append(0.0)
                    avg_total_early_returns_lc[policy] = totals

                for policy in policies:
                    for rps_idx in range(len(rps_values)):
                        for rt in all_types_lc:
                            vals = []
                            for i in range(repeats):
                                if i >= len(policy_early_returns_last_child_by_type):
                                    continue
                                per_rps = (
                                    policy_early_returns_last_child_by_type[i]
                                    .get(api, {})
                                    .get(policy, [])
                                )
                                if (
                                    rps_idx < len(per_rps)
                                    and per_rps[rps_idx] is not None
                                    and rt in per_rps[rps_idx]
                                ):
                                    vals.append(float(per_rps[rps_idx][rt]))
                            if vals:
                                avg_breakdown_lc[policy][rps_idx][rt] = sum(vals) / len(
                                    vals
                                )

                output_path = os.path.join(output_dir, f"early_return_last_child_{api}.png")
                futures.append(
                    executor.submit(
                        _plot_early_return_breakdown,
                        output_path,
                        policies=policies,
                        rps_values=rps_values,
                        policy_total_early_returns=avg_total_early_returns_lc,
                        policy_early_returns_breakdown=avg_breakdown_lc,
                        title=f"Average early-return requests breakdown by Last Child Service::Method (averaged over {repeats} run(s))",
                    )
                )

        # Submit averaged SLO miss breakdown plot
        output_dir = args.output_dir
        for api in apis:
            if api == "ALL":
                # Average breakdown per (policy, rps, request_type)
                avg_total_slo_misses = {}
                avg_breakdown = {
                    policy: [dict() for _ in range(len(rps_values))]
                    for policy in policies
                }

                # Collect all request types present anywhere
                all_types = set()
                for i in range(repeats):
                    if i < len(policy_slo_misses_by_type):
                        for policy in policies:
                            per_rps = (
                                policy_slo_misses_by_type[i]
                                .get(api, {})
                                .get(policy, [])
                            )
                            for d in per_rps:
                                all_types.update((d or {}).keys())

                # Average totals
                for policy in policies:
                    totals = []
                    for rps_idx in range(len(rps_values)):
                        vals = []
                        for i in range(repeats):
                            if i < len(policy_total_slo_misses):
                                per_rps = (
                                    policy_total_slo_misses[i]
                                    .get(api, {})
                                    .get(policy, [])
                                )
                                if rps_idx < len(per_rps):
                                    vals.append(float(per_rps[rps_idx] or 0.0))
                        if vals:
                            totals.append(sum(vals) / len(vals))
                        else:
                            totals.append(0.0)
                    avg_total_slo_misses[policy] = totals

                # Average breakdown
                for policy in policies:
                    for rps_idx in range(len(rps_values)):
                        for rt in all_types:
                            vals = []
                            for i in range(repeats):
                                if i >= len(policy_slo_misses_by_type):
                                    continue
                                per_rps = (
                                    policy_slo_misses_by_type[i]
                                    .get(api, {})
                                    .get(policy, [])
                                )
                                if (
                                    rps_idx < len(per_rps)
                                    and per_rps[rps_idx] is not None
                                    and rt in per_rps[rps_idx]
                                ):
                                    vals.append(float(per_rps[rps_idx][rt]))
                            if vals:
                                avg_breakdown[policy][rps_idx][rt] = sum(vals) / len(
                                    vals
                                )

                output_path = os.path.join(output_dir, f"slo_miss_{api}.png")
                futures.append(
                    executor.submit(
                        _plot_slo_miss_breakdown,
                        output_path,
                        policies=policies,
                        rps_values=rps_values,
                        policy_total_slo_misses=avg_total_slo_misses,
                        policy_slo_misses_by_type=avg_breakdown,
                        title=f"Average SLO-miss requests breakdown by API (averaged over {repeats} run(s))",
                        request_type_color_mapping=request_type_color_mapping,
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
