import numpy as np
from util import read_data

if __name__ == "__main__":
    for dir in (
        "synthetic/sanity_check_constant",
        "synthetic/sanity_check_random1",
        "synthetic/sanity_check_random2",
        "synthetic/sanity_check_random3",
    ):
        print(dir)
        print(20 * "-")
        apis, policies, rps_values, results = read_data(f"data/{dir}")
        for policy in policies:
            print(f"{5 * '*'} {policy} {5 * '*'}")
            for rps in rps_values:
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
        print("\n\n\n", end="")
