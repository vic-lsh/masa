from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, MODES, plot_cdf, plot_pdf  # type: ignore

rps_list = [300, 325, 350, 375, 400, 425, 450]

mode_to_graph_id_to_tail_latency: Dict[str, Dict[str, List[Tuple[int, int]]]] = dict()

for mode in MODES:
    graph_id_to_tail_latency = DefaultDict(list)
    for rps in rps_list:
        file = f"r{rps}_{mode}.csv"
        df = pd.read_csv(file)
        for graph_id in GRAPH_IDS:
            df_filtered = df[df["graph_id"] == graph_id]
            p99 = round(df_filtered["latency"].quantile(0.99))
            graph_id_to_tail_latency[graph_id].append((rps, p99))
    mode_to_graph_id_to_tail_latency[mode] = graph_id_to_tail_latency

print(mode_to_graph_id_to_tail_latency)
