from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, MODES, plot_cdf, plot_pdf  # type: ignore

repeats = 2
graph_id = GRAPH_IDS[0]
rps = 500

for mode in MODES:
    e2e_repeats: List[Tuple[str, List[int]]] = []
    for r in range(repeats):
        file = f"r{rps}_hotel_client_bench_{r}.csv"
        df = pd.read_csv(file)
        for graph_id in GRAPH_IDS:
            df_filtered = df[df["graph_id"] == graph_id]
            e2e_repeats.append((f"repeat_{r}", list(df_filtered["latency"])))

    for graph_id in GRAPH_IDS:
        plot_cdf(
            e2e_repeats,
            f"E2E Latency CDF (rps={rps})",
            f"fig_{mode}_r{rps}_e2e_cdf.png",
        )
        plot_pdf(
            e2e_repeats,
            f"E2E Latency PDF (rps={rps})",
            f"fig_{mode}_r{rps}_e2e_pdf.png",
        )
