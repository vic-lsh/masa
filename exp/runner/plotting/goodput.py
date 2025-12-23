import os

import matplotlib.pyplot as plt
import numpy as np
from typing import Optional

from .util import parse_args, prepare_output_dir, read_data


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
    df["met_slo"] = df["error"] == "/None"
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    return df["met_slo"].sum() / duration_us * s_to_us


def compute_goodput_time_series(df, bucket_seconds: float = 1.0):
    """Return per-second goodput counts for the provided request dataframe."""
    if bucket_seconds <= 0:
        raise ValueError("bucket_seconds must be positive")

    if df.empty:
        return np.array([]), np.array([])

    bucket_us = int(bucket_seconds * 10**6)
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()

    if end <= start:
        end = start + bucket_us

    bins = np.arange(start, end + bucket_us, bucket_us)
    if len(bins) < 2:
        bins = np.array([start, start + bucket_us])

    met_mask = df["error"] == "/None"
    if met_mask.any():
        completion_times = df.loc[met_mask, "start_at"] + df.loc[met_mask, "latency"]
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
    plt.tight_layout()
    plt.savefig(output_path, dpi=300)
    plt.close(fig)


def generate_plots(args) -> None:
    prepare_output_dir(args)

    repeats, apis, policies, rps_values, results = read_data(
        args.config_dir, args.data_dir
    )

    policy_goodputs = []
    for i in range(repeats):
        policy_goodputs.append({})
        output_dir = os.path.join(args.output_dir, str(i))
        for api in apis:
            data = results[i][api]

            policy_goodputs[i][api] = {
                policy: [compute_goodput(data[policy][rps]) for rps in rps_values]
                for policy in policies
            }

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
                    policy_goodputs[i][api][policy],
                    bar_width,
                    label=policy,
                    color=color,
                )

                # Add labels on top of bars
                # for bar in bars:
                #     height = bar.get_height()
                #     ax.annotate(
                #         f"{height:.2f}",
                #         xy=(bar.get_x() + bar.get_width() / 2, height),
                #         xytext=(0, 3),  # 3 points vertical offset
                #         textcoords="offset points",
                #         ha="center",
                #         va="bottom",
                #     )

            # Add labels and title
            ax.set_xlabel("Requests Per Second (RPS)")
            ax.set_ylabel("Goodput (requests meeting SLO per second)")
            ax.set_title(f"Goodput Comparison by Policy and RPS for {api} API")
            ax.set_xticks(index)
            ax.set_xticklabels([str(rps) for rps in rps_values])
            ax.legend()

            plt.grid(axis="y", linestyle="--", alpha=0.7)
            plt.tight_layout()
            plt.savefig(
                os.path.join(output_dir, f"policy_goodput_comparison_{api}.png"),
                dpi=300,
            )
            plt.close()

            for policy in policies:
                policy_dir = os.path.join(output_dir, policy)
                os.makedirs(policy_dir, exist_ok=True)
                for rps in rps_values:
                    df = data[policy][rps]
                    plot_goodput_time_series(
                        df,
                        os.path.join(
                            policy_dir, f"goodput_time_series_{rps}rps_{api}.png"
                        ),
                        bucket_seconds=1.0,
                        title=f"Goodput Time Series for {api} - {policy} - {rps} RPS",
                    )

    output_dir = args.output_dir
    # averaged goodput
    for api in apis:
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

        plt.grid(axis="y", linestyle="--", alpha=0.7)
        plt.tight_layout()
        plt.savefig(
            os.path.join(output_dir, f"policy_goodput_comparison_{api}_averaged.png"),
            dpi=300,
        )
        plt.close()


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)
