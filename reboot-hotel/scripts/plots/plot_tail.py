import json
from typing import *

import pandas as pd
from plot_core import (  # type: ignore
    GRAPH_IDS,
    parse_args,
    plot_tail_bar,
    plot_tail_line,
)

args = parse_args()
cfg = json.load(open(f"{args.path}/gen_config.json"))
ms = 1e3

for r in range(cfg["Repeats"]):
    rps_to_results: List[Dict[str, Any]] = []

    for rps in cfg["Rps"]:
        file = f"{args.path}/r{rps}_{r}.csv"
        df = pd.read_csv(file)
        for graph_id in GRAPH_IDS:
            result = {}
            result["rps"] = rps
            result["graph_id"] = graph_id

            df_filtered = df[
                (df["graph_id"] == graph_id)
                & (df["error"].isin(["/None", "/LGMiss", "/LGTimeout"]))
            ]
            result["mean"] = round(df_filtered["latency"].mean() / ms)
            result["p90"] = round(df_filtered["latency"].quantile(0.9) / ms)
            result["p95"] = round(df_filtered["latency"].quantile(0.95) / ms)
            result["p99"] = round(df_filtered["latency"].quantile(0.99) / ms)

            rps_to_results.append(result)

    plot_tail_line(
        rps_to_results, f"{args.path}/fig_tail_rps_line_{r}.png", args.mode, args.alias
    )
    plot_tail_bar(
        rps_to_results, f"{args.path}/fig_tail_rps_bar_{r}.png", args.mode, args.alias
    )
