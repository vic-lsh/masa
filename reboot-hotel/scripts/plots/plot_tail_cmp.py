import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_tail_cmp_bar  # type: ignore

args = parse_args()
ms = 1e3
r = 0
rps_to_results: List[Dict[str, Any]] = []

for mode in args.modes:
    path = f"{args.snippets}/{mode}"
    cfg = json.load(open(f"{path}/gen_config.json"))

    for rps in cfg["Rps"]:
        file = f"{path}/{args.data}/r{rps}_{r}.csv"
        df = pd.read_csv(file)

        result = {}
        result["rps"] = rps

        df_filtered = df[df["error"] == "/None"]
        if df_filtered.empty:
            print(f"Empty data for rps: {rps}")
            result[f"mean_{mode}"] = 0
            result[f"p90_{mode}"] = 0
            result[f"p95_{mode}"] = 0
            result[f"p99_{mode}"] = 0
        else:
            result[f"mean_{mode}"] = round(df_filtered["latency"].mean() / ms)
            result[f"p90_{mode}"] = round(df_filtered["latency"].quantile(0.9) / ms)
            result[f"p95_{mode}"] = round(df_filtered["latency"].quantile(0.95) / ms)
            result[f"p99_{mode}"] = round(df_filtered["latency"].quantile(0.99) / ms)

        rps_to_results.append(result)

modes_str = "_".join(args.modes)
for tail in ["mean", "p90", "p95", "p99"]:
    plot_tail_cmp_bar(
        args.modes,
        tail,
        rps_to_results,
        f"{args.path}/fig_{tail}_total_rps_cmp_{modes_str}_{r}.png",
    )
