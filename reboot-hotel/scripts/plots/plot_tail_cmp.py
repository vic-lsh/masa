import json
from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, parse_args, plot_tail_cmp_bar  # type: ignore

args = parse_args()
ms = 1e3
r = 0
rps_to_results: List[Dict[str, Any]] = []

for mode in args.modes:
    snippet_path = f"{args.snippets}/{mode}"
    cfg = json.load(open(f"{snippet_path}/gen_config.json"))

    for rps in cfg["Rps"]:
        file = f"{snippet_path}/r{rps}_{r}.csv"
        df = pd.read_csv(file)
        for graph_id in GRAPH_IDS:
            result = {}
            result["rps"] = rps
            result["graph_id"] = graph_id
            df_filtered = df[
                (df["graph_id"] == graph_id) & (df["error"].isin(["/None", "/LGMiss"]))
            ]
            result[f"mean_{mode}"] = round(df_filtered["latency"].mean() / ms)
            result[f"p90_{mode}"] = round(df_filtered["latency"].quantile(0.9) / ms)
            result[f"p95_{mode}"] = round(df_filtered["latency"].quantile(0.95) / ms)
            result[f"p99_{mode}"] = round(df_filtered["latency"].quantile(0.99) / ms)
            rps_to_results.append(result)

for tail in ["mean", "p90", "p95", "p99"]:
    plot_tail_cmp_bar(
        rps_to_results, f"{args.path}/fig_{tail}_rps_bar_{r}.png", args.modes, tail
    )
