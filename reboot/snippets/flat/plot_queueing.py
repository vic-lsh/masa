from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 1600
modes = ["masa", "fifo"]
results_raw = {}
sh_i1 = "say_hello_i1"
sh_i2 = "say_hello_i2"

for mode in modes:
    file = f"tmp_{mode}_log.txt"
    id_to_spans: Dict[int, List[Dict]] = {}

    with open(file) as f:
        lines = f.readlines()
        for line in lines:
            if sh_i1 in line or sh_i2 in line:
                splits = line.split()[-1].split(",")
                span = splits[0]
                id = int(splits[1])
                elapse = int(splits[2])
                latency = int(splits[3])
                id_to_spans.setdefault(id, []).append(
                    {"span": span, "elapse": elapse, "latency": latency}
                )

    results_raw[mode] = id_to_spans


def plot_cdf(results: Dict[str, List[int]], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    for mode in modes:
        latencies = [r / 1e3 for r in results[mode]]
        latencies.sort()
        cdf = np.arange(1, len(latencies) + 1) / len(latencies)
        plt.plot(latencies, cdf, label=mode)

    plt.xlabel("Latency (ms)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


def plot_pdf(results: Dict[str, List[int]], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    for mode in modes:
        latencies = [r / 1e3 for r in results[mode]]
        plt.hist(latencies, bins=1000, density=True, histtype="step", label=mode)

    plt.xlabel("Latency (ms)")
    plt.ylabel("PDF")
    plt.title(title)
    plt.legend()
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


# results_second_hop_queueing: Dict[str, List[int]] = {"masa": [], "fifo": []}
# for mode in modes:
#     n_dp = 0
#     id_to_spans = results_raw[mode]
#     for id, spans in id_to_spans.items():
#         if len(spans) == 2:
#             n_dp += 1
#             # [NOTE] Depending on the definition of "latency" and "elapse".
#             # queueing_latency = spans[1]["latency"] - spans[1]["elapse"] - spans[0]["latency"]
#             queueing_latency = spans[1]["latency"] - spans[0]["latency"]
#             results_second_hop_queueing[mode].append(queueing_latency)
#     pctl_dp = n_dp / len(id_to_spans)
#     print(f"{mode}: {round(pctl_dp*100)}% data points")

# plot_cdf(
#     results_second_hop_queueing,
#     f"Queueing Latency at Second Hop (rps={rps})",
#     "fig_second_hop_queueing_cdf.png",
# )

# plot_pdf(
#     results_second_hop_queueing,
#     f"Queueing Latency at Second Hop (rps={rps})",
#     "fig_second_hop_queueing_pdf.png",
# )


def plot_ratio_cdf(results: List[float], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    results.sort()
    cdf = np.arange(1, len(results) + 1) / len(results)

    plt.plot(results, cdf)

    pois = [100, 200, 300, 400, 500]
    for poi in pois:
        # plt.axvline(x=poi, color="r", linestyle="--")
        if results[-1] >= poi:
            y_value = next((y for x, y in zip(results, cdf) if x >= poi), None)
            if y_value is not None:
                plt.axhline(
                    y=y_value,
                    color="forestgreen",
                    linestyle="--",
                    label=f"CDF at x={poi}: {y_value:.2f}",
                )

    plt.legend()

    plt.xlabel("Latency Ratio (%)")
    plt.ylabel("CDF")
    plt.title(title)
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


def plot_ratio_pdf(results: List[float], title: str, fig_name: str):
    fig = plt.figure(figsize=(10, 6))

    plt.hist(results, bins=1000, density=True, histtype="step", label="masa / fifo")

    plt.legend()

    plt.xlabel("Latency Ratio (%)")
    plt.ylabel("PDF")
    plt.title(title)
    plt.grid(True)
    plt.savefig(fig_name)
    plt.show()


# results_second_hop_queueing_ratio: List[float] = []
# for i in range(len(results_second_hop_queueing["masa"])):
#     masa = results_second_hop_queueing["masa"][i]
#     fifo = results_second_hop_queueing["fifo"][i]
#     ratio = masa / fifo * 100
#     results_second_hop_queueing_ratio.append(ratio)

# plot_ratio_cdf(
#     results_second_hop_queueing_ratio,
#     f"Queueing Latency Ratio at Second Hop (masa / fifo) (rps={rps})",
#     "fig_second_hop_queueing_ratio_cdf.png",
# )

# plot_ratio_pdf(
#     results_second_hop_queueing_ratio,
#     f"Queueing Latency Ratio at Second Hop (masa / fifo) (rps={rps})",
#     "fig_second_hop_queueing_ratio_pdf.png",
# )
