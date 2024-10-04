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

results_e2e: Dict[str, List[int]] = {}
for mode in MODES:
    """
    file = f"r{rps}_{mode}_filtered.csv"
    """
    file = f"r{rps}_{mode}.csv"
    df = pd.read_csv(file)
    results_e2e[mode] = list(df["latency"])

plot_cdf(results_e2e, f"E2E Latency CDF (rps={rps})", f"fig_e2e_cdf.png")

plot_pdf(results_e2e, f"E2E Latency PDF (rps={rps})", f"fig_e2e_pdf.png")

exit()

plot_2d_histogram(
    results_e2e,
    f"E2E Latency 2D Histogram (rps={rps})",
    f"fig_e2e_2d_hist.png",
)


results_ratio: List[float] = []
for i in range(len(results_e2e["queue_edf"])):
    masa = results_e2e["queue_edf"][i]
    fifo = results_e2e["queue_fifo"][i]
    ratio = masa / fifo * 100
    results_ratio.append(ratio)

plot_ratio_cdf(
    results_ratio,
    f"E2E Latency Ratio CDF (masa / fifo) (rps={rps})",
    f"fig_e2e_ratio_cdf.png",
)

plot_ratio_pdf(
    results_ratio,
    f"E2E Latency Ratio PDF (masa / fifo) (rps={rps})",
    f"fig_e2e_ratio_pdf.png",
)


results_slower: Dict[str, List[int]] = {"queue_edf": [], "queue_fifo": []}
results_faster: Dict[str, List[int]] = {"queue_edf": [], "queue_fifo": []}

for i in range(len(results_e2e["queue_edf"])):
    masa = results_e2e["queue_edf"][i]
    fifo = results_e2e["queue_fifo"][i]
    if masa > fifo:
        results_slower["queue_edf"].append(masa)
        results_slower["queue_fifo"].append(fifo)
    else:
        results_faster["queue_edf"].append(masa)
        results_faster["queue_fifo"].append(fifo)


plot_cdf(
    results_slower,
    f"E2E Slower Latency CDF (masa > fifo) (rps={rps})",
    f"fig_e2e_slower_cdf.png",
)
plot_pdf(
    results_slower,
    f"E2E Slower Latency PDF (masa > fifo) (rps={rps})",
    f"fig_e2e_slower_pdf.png",
)
plot_2d_histogram(
    results_slower,
    f"E2E Slower Latency 2D Histogram (masa > fifo) (rps={rps})",
    f"fig_e2e_slower_2d_hist.png",
)

plot_cdf(
    results_faster,
    f"E2E Faster Latency CDF (masa < fifo) (rps={rps})",
    f"fig_e2e_faster_cdf.png",
)
plot_pdf(
    results_faster,
    f"E2E Faster Latency PDF (masa < fifo) (rps={rps})",
    f"fig_e2e_faster_pdf.png",
)
plot_2d_histogram(
    results_faster,
    f"E2E Faster Latency 2D Histogram (masa < fifo) (rps={rps})",
    f"fig_e2e_faster_2d_hist.png",
)
