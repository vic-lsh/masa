import os
from collections import OrderedDict

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd
import seaborn as sns
from tabulate import tabulate
from util import read_data


def handler_latency_stats(results, policy, rps):
    df = results["a"][policy][rps]
    no_timeout = df[(df["error"] == "/None") | (df["error"] == "/ClientMiss")]
    fraction_timeout = 1 - len(no_timeout) / len(df)
    print(f"{fraction_timeout * 100:.3f}% of requests timed out")
    for col in ("frontend_latency", "child1_latency", "child2_latency"):
        print(f"avg: {np.average(no_timeout[col])}, std: {np.std(no_timeout[col])}")
        # plt.figure(figsize=(12, 8))
        # plt.title(f"{col} histogram - {rps} RPS")
        # sns.histplot(
        #     data=no_timeout,
        #     x=col,
        # )
        # dir = f"/tmp/plots/{policy}"
        # os.makedirs(dir, exist_ok=True)
        # plt.savefig(f"{dir}/{col}_{rps}rps.png", dpi=300)
        # plt.close()


def general_stats(results, policy, rps):
    df = results["ALL"][policy][rps]
    output = {}

    # # time between requests
    # start_times = np.array(sorted(df["start_at"]))
    # avg_delta = np.average(start_times[1:] - start_times[:-1])

    # non-timeout avg latency and #timeouts
    timeout_mask = df["error"] == "/ClientTimeout"
    non_timeout_latencies = np.array(df[~timeout_mask]["latency"])
    output["latency_avg"] = np.average(non_timeout_latencies)
    output["latency_std"] = np.std(non_timeout_latencies)
    output["timeouts"] = timeout_mask.sum() / len(df)

    # rps
    start = df["start_at"].min()
    end = df["start_at"].max()
    duration_us = end - start
    s_to_us = 10**6
    output["r_rps"] = len(df) / duration_us * s_to_us

    print(f"rps = {rps}")
    print("\n".join(map(lambda t: f"{t[0]} = {t[1]}", output.items())))
    print()


def queueing_latency(results, rps_values, policy):
    # x = rps_values
    # y = []
    for rps in rps_values:
        print(f"rps = {rps}")
        df = results["Reservation"][policy][rps]
        handler_latencies = [
            "get_mc_client",
            "reservation_total_mc_latency",
            "reservation_total_mongo_latency",
            "capacity_total_mc_latency",
            "capacity_total_mongo_latency",
            "mc_bulk_insert_latency",
            "reservation_bulk_insert_latency",
        ]
        df["diff"] = df["reservation_latency"] - df[handler_latencies].sum(axis=1)
        df["remaining_handler_latency"] = df["latency"] - (
            df["check_user_latency"]
            + df["queueing_latency"]
            + df["reservation_latency"]
        )
        n_all = len(df)
        df = df[((df["error"] == "/None") | (df["error"] == "/ClientMiss"))]
        fraction_timeout = 1 - len(df) / n_all
        print(f"{fraction_timeout * 100:.3f}% of all reservation requests timed out")
        df = df[df["queueing_latency"] > 0]
        fraction_queueing_latency = len(df) / n_all
        print(
            f"{fraction_queueing_latency * 100:.3f}% ({len(df)}) of requests have queueing_latency data"
        )
        print(f"sum e2e latencies: {df["latency"].sum() / 10**6}")
        rows = [
            "latency",
            "check_user_latency",
            "queueing_latency",
            "reservation_latency",
            "remaining_handler_latency",
            "diff",
            "reservation_mc_misses",
            "reservation_total_mongo_latency",
            "capacity_mc_misses",
            "capacity_total_mongo_latency",
        ]
        data = OrderedDict(
            [("metric", []), ("average", []), ("std", []), ("p80", []), ("p90", []), ("p99", []), ("max", [])]
        )
        for row in rows:
            data["metric"].append(row)
            data["average"].append(np.average(df[row]))
            data["std"].append(np.std(df[row]))
            data["p80"].append(df[row].quantile(0.80))
            data["p90"].append(df[row].quantile(0.90))
            data["p99"].append(df[row].quantile(0.99))
            data["max"].append(df[row].max())
        df = pd.DataFrame(data)
        df = df.set_index("metric")
        print(tabulate(df, headers=df.columns, tablefmt="fancy_grid", showindex=True))
        print()
        # y.append(avg_queueing_latency / 1000)
    # plt.plot(x, y, "o-", label=policy)


if __name__ == "__main__":
    for exp in ("reservation_analysis_locks",):
        print(exp)
        print(20 * "-")
        repeats, apis, policies, rps_values, results = read_data(
            f"data/in/{exp}", f"data/out/{exp}"
        )

        for i in range(repeats):
            print(f"{10 * '-'} iteration {i} {10 * '-'}")
            # plt.figure(figsize=(12, 8))
            for policy in policies:
                print(f"{5 * '*'} {policy} {5 * '*'}")
                queueing_latency(results[i], rps_values, policy)
            # plt.title(exp + f"_{i}")
            # plt.xlabel("rps")
            # plt.ylabel("avg. queueing latency (ms)")
            # plt.legend()
            # plt.show()
            # plt.close()
            # print("\n\n\n", end="")
