import json
from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, parse_args, plot_goodput_cmp_bar  # type: ignore

args = parse_args()
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
                (df["graph_id"] == graph_id)
                & (df["error"] == "/None")
                & (df["latency"] <= cfg["Slo"])
            ]
            result[f"goodput_{mode}"] = round(len(df_filtered) / cfg["DurationSecs"])
            rps_to_results.append(result)

plot_goodput_cmp_bar(
    rps_to_results, f"{args.path}/fig_goodput_rps_bar_{r}.png", args.modes
)
