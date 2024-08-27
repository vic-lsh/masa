from typing import *

import pandas as pd
from plot_core import (  # type: ignore
    MODES,
    plot_2d_histogram,
    plot_cdf,
    plot_pdf,
    plot_ratio_cdf,
    plot_ratio_pdf,
)

rps = 50

results_all: Dict[str, List[int]] = {}
results_infra: Dict[str, List[int]] = {}
results_req: Dict[str, List[int]] = {}

for mode in MODES:
    file = f"r{rps}_{mode}_all_queueing.csv"
    df = pd.read_csv(file)
    results_all[mode] = list(df["queueing"])

    file = f"r{rps}_{mode}_infra_queueing.csv"
    df = pd.read_csv(file)
    results_infra[mode] = list(df["queueing"])

    file = f"r{rps}_{mode}_req_queueing.csv"
    df = pd.read_csv(file)
    results_req[mode] = list(df["queueing"])

plot_cdf(
    results_all,
    f"All Task Queueing Latency (rps={rps})",
    "fig_all_queueing_cdf.png",
)

plot_pdf(
    results_all,
    f"All Task Queueing Latency (rps={rps})",
    "fig_all_queueing_pdf.png",
)

plot_cdf(
    results_infra,
    f"Infra Task Queueing Latency (rps={rps})",
    "fig_infra_queueing_cdf.png",
)

plot_pdf(
    results_infra,
    f"Infra Task Queueing Latency (rps={rps})",
    "fig_infra_queueing_pdf.png",
)

plot_cdf(
    results_req,
    f"Req Task Queueing Latency (rps={rps})",
    "fig_req_queueing_cdf.png",
)

plot_pdf(
    results_req,
    f"Req Task Queueing Latency (rps={rps})",
    "fig_req_queueing_pdf.png",
)

"""
exit()

plot_2d_histogram(
    results_queueing,
    f"Queueing Latency at Second Hop 2D Histogram (rps={rps})",
    "fig_queueing_second_hop_2d_hist.png",
)


results_queueing_ratio: List[float] = []
for i in range(len(results_queueing["masa"])):
    masa = results_queueing["masa"][i]
    fifo = results_queueing["fifo"][i]
    ratio = masa / fifo * 100
    results_queueing_ratio.append(ratio)

plot_ratio_cdf(
    results_queueing_ratio,
    f"Queueing Latency Ratio at Second Hop (masa / fifo) (rps={rps})",
    "fig_queueing_ratio_second_hop_cdf.png",
)

plot_ratio_pdf(
    results_queueing_ratio,
    f"Queueing Latency Ratio at Second Hop (masa / fifo) (rps={rps})",
    "fig_queueing_ratio_second_hop_pdf.png",
)


results_queueing_slower: Dict[str, List[int]] = {"masa": [], "fifo": []}
results_queueing_faster: Dict[str, List[int]] = {"masa": [], "fifo": []}

for i in range(len(results_queueing["masa"])):
    masa = results_queueing["masa"][i]
    fifo = results_queueing["fifo"][i]
    if masa > fifo:
        results_queueing_slower["masa"].append(masa)
        results_queueing_slower["fifo"].append(fifo)
    else:
        results_queueing_faster["masa"].append(masa)
        results_queueing_faster["fifo"].append(fifo)

plot_cdf(
    results_queueing_slower,
    f"Queueing Slower Latency at Second Hop (masa > fifo) (rps={rps})",
    "fig_queueing_slower_second_hop_cdf.png",
)
plot_pdf(
    results_queueing_slower,
    f"Queueing Slower Latency at Second Hop (masa > fifo) (rps={rps})",
    "fig_queueing_slower_second_hop_pdf.png",
)
plot_2d_histogram(
    results_queueing_slower,
    f"Queueing Slower Latency at Second Hop 2D Histogram (masa > fifo) (rps={rps})",
    "fig_queueing_slower_second_hop_2d_hist.png",
)

plot_cdf(
    results_queueing_faster,
    f"Queueing Faster Latency at Second Hop (masa < fifo) (rps={rps})",
    "fig_queueing_faster_second_hop_cdf.png",
)
plot_pdf(
    results_queueing_faster,
    f"Queueing Faster Latency at Second Hop (masa < fifo) (rps={rps})",
    "fig_queueing_faster_second_hop_pdf.png",
)
plot_2d_histogram(
    results_queueing_faster,
    f"Queueing Faster Latency at Second Hop 2D Histogram (masa < fifo) (rps={rps})",
    "fig_queueing_faster_second_hop_2d_hist.png",
)
"""
