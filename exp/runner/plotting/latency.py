import os

import matplotlib.pyplot as plt
import numpy as np
# pyrefly: ignore  # import-error
import seaborn as sns
# pyrefly: ignore  # import-error
from util import parse_args, prepare_output_dir, read_data


def generate_plots(args) -> None:
    prepare_output_dir(args)

    repeats, apis, policies, rps_values, results = read_data(
        args.config_dir, args.data_dir
    )

    MS_TO_US = 10**3
    for i in range(repeats):
        output_dir = os.path.join(args.output_dir, str(i))
        for api in apis:
            data = results[i][api]
            # microseconds to milliseconds
            for rps in rps_values:
                for policy in policies:
                    df = data[policy][rps]
                    df["latency"] /= MS_TO_US
                    df["slo"] /= MS_TO_US
                    df["start_at"] /= MS_TO_US
                    df["deadline"] /= MS_TO_US

            slo = data[policies[0]][rps_values[0]]["slo"].max()
            max_y = slo * 4

            # plot CDF of latency distribution
            for rps in rps_values:
                plt.figure(figsize=(12, 8))

                for policy in policies:
                    df = data[policy][rps]

                    latencies = sorted(df["latency"].values)
                    percentiles = np.linspace(0, 100, len(latencies))

                    plt.plot(percentiles, latencies, label=f"{policy}")

                # Add labels and title
                plt.xlabel("Percentile (%)")
                plt.ylabel("Latency (milliseconds)")
                plt.title(f"Latency Distribution for {api} API - {rps} RPS")
                plt.grid(True, alpha=0.3)
                plt.legend()
                plt.ylim(top=max_y)
                # Save the plot
                plt.savefig(
                    f"{output_dir}/latency_distribution_{rps}rps_{api}.png", dpi=300
                )

                # # Optional: Add log scale version for better visibility of tail latencies
                # plt.yscale("log")
                # plt.title(f"Latency Distribution (Log Scale) for {api} API - {rps} RPS")
                # plt.savefig(
                #     f"{output_dir}/latency_distribution_{rps}rps_{api}_log.png", dpi=300
                # )
                plt.close()

            # TODO: DRY
            # plot CDF of GOODPUT latency distribution
            for rps in rps_values:
                plt.figure(figsize=(12, 8))

                for policy in policies:
                    df = data[policy][rps]
                    df = df[df["error"] == "/None"]

                    latencies = sorted(df["latency"].values)
                    percentiles = np.linspace(0, 100, len(latencies))

                    plt.plot(percentiles, latencies, label=f"{policy}")

                # Add labels and title
                plt.xlabel("Percentile (%)")
                plt.ylabel("Latency (milliseconds)")
                plt.title(f"Goodput Latency Distribution for {api} API - {rps} RPS")
                plt.grid(True, alpha=0.3)
                plt.ylim(top=max_y)
                plt.legend()

                # Save the plot
                plt.savefig(
                    f"{output_dir}/goodput_latency_distribution_{rps}rps_{api}.png",
                    dpi=300,
                )

                # # Optional: Add log scale version for better visibility of tail latencies
                # plt.yscale("log")
                # plt.title(f"Latency Distribution for {api} API (Log Scale) - {rps} RPS")
                # plt.savefig(
                #     f"{output_dir}/latency_distribution_{rps}rps_{api}_log.png", dpi=300
                # )
                plt.close()

            # plot histogram
            for rps in rps_values:
                for policy in policies:
                    plt.figure(figsize=(12, 8))
                    df = data[policy][rps]
                    df["met_slo"] = df["error"] == "/None"

                    sns.histplot(
                        data=df,
                        x="latency",
                        hue="met_slo",
                        hue_order=[True, False],
                    )

                    # Add labels and title
                    plt.xlabel("Latency (milliseconds)")
                    plt.title(f"Latency Histogram for {api} API - {policy} - {rps} RPS")
                    dir = os.path.join(output_dir, policy)
                    os.makedirs(dir, exist_ok=True)
                    plt.savefig(
                        os.path.join(dir, f"latency_histogram_{rps}rps_{api}.png"),
                        dpi=300,
                    )
                    plt.close()

            # Create a summary plot for 99th percentile latencies
            plt.figure(figsize=(12, 6))
            for policy in policies:
                p99_values = []
                for rps in rps_values:
                    df = data[policy][rps]
                    p99_latency = df["latency"].quantile(0.99)
                    p99_values.append(p99_latency)
                plt.plot(rps_values, p99_values, "o-", label=f"{policy}")

            plt.xlabel("Requests Per Second (RPS)")
            plt.ylabel("p99 latency (milliseconds)")
            plt.title(f"p99 latency by policy and RPS for {api} API")
            plt.grid(True, alpha=0.3)
            plt.legend()
            plt.ylim(top=max_y)
            plt.savefig(f"{output_dir}/p99_latency_rps_{api}.png", dpi=300)
            plt.close()

    # averaged pX latencies
    percentiles = [0.80, 0.90, 0.99]
    for percentile in percentiles:
        for api in apis:
            plt.figure(figsize=(12, 6))
            for policy in policies:
                averaged_percentile = np.zeros(len(rps_values))
                for i in range(repeats):
                    data = results[i][api]
                    percentile_values = []
                    for rps in rps_values:
                        df = data[policy][rps]
                        percentile_latency = df["latency"].quantile(percentile)
                        percentile_values.append(percentile_latency)
                    averaged_percentile += np.array(percentile_values)
                plt.plot(rps_values, averaged_percentile / repeats, "o-", label=f"{policy}")

            p = int(percentile * 100)
            plt.xlabel("Requests Per Second (RPS)")
            plt.ylabel(f"average p{p} latency (milliseconds)")
            plt.title(
                f"p{p} latency by policy and RPS for {api} API averaged over {repeats} runs"
            )
            plt.grid(True, alpha=0.3)
            plt.legend()
            for max_y in [int(slo * 4), 1000]:
                plt.ylim(bottom=0, top=max_y)
                plt.savefig(
                    f"{args.output_dir}/p{p}_latency_rps_{api}_averaged_maxy-{max_y}.png",
                    dpi=300,
                )
            plt.close()


if __name__ == "__main__":
    args = parse_args()
    generate_plots(args)
