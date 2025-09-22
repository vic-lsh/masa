import os

import matplotlib.pyplot as plt
import numpy as np
# pyrefly: ignore  # import-error
from util import parse_args, prepare_output_dir, read_data


def compute_goodput(df):
    df["met_slo"] = df["error"] == "/None"
    start = df["start_at"].min()
    end = (df["start_at"] + df["latency"]).max()
    duration_us = end - start
    s_to_us = 10**6

    return df["met_slo"].sum() / duration_us * s_to_us


def generate_plots(args):
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

            # set width of bars
            bar_width = 0.12
            index = np.arange(len(rps_values))

            # Create bars
            for j, policy in enumerate(policies):
                offset = (j - len(policies) / 2 + 0.5) * bar_width
                bars = ax.bar(
                    index + offset,
                    policy_goodputs[i][api][policy],
                    bar_width,
                    label=policy,
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

    output_dir = args.output_dir
    # averaged goodput
    for api in apis:
        fig, ax = plt.subplots(figsize=(12, 6))

        bar_width = 0.12
        index = np.arange(len(rps_values))

        for j, policy in enumerate(policies):
            average_goodput = (
                sum(np.array(policy_goodputs[i][api][policy]) for i in range(repeats))
                / repeats
            )
            offset = (j - len(policies) / 2 + 0.5) * bar_width
            bars = ax.bar(
                index + offset,
                average_goodput,
                bar_width,
                label=policy,
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
