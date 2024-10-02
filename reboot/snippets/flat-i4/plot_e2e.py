from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, MODES, plot_cdf, plot_pdf  # type: ignore

rps = 400

graph_id_to_e2e: Dict[str, Dict[str, List[int]]] = DefaultDict(dict)
for mode in MODES:
    file = f"r{rps}_{mode}.csv"
    df = pd.read_csv(file)
    for graph_id in GRAPH_IDS:
        df_filtered = df[df["graph_id"] == graph_id]
        graph_id_to_e2e[graph_id][mode] = list(df_filtered["latency"])

for graph_id in GRAPH_IDS:
    e2e = graph_id_to_e2e[graph_id]
    plot_cdf(e2e, f"E2E Latency CDF (rps={rps})", f"fig_{graph_id}_e2e_cdf.png")
    plot_pdf(e2e, f"E2E Latency PDF (rps={rps})", f"fig_{graph_id}_e2e_pdf.png")
