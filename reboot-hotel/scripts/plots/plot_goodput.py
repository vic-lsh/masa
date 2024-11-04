import json
from typing import *

import pandas as pd
from plot_core import (  # type: ignore
    GRAPH_IDS,
    parse_args,
    plot_goodput_bar,
    plot_goodput_line,
)

args = parse_args()
cfg = json.load(open(f"{args.path}/gen_config.json"))

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
                & (df["error"] == "/None")
                & (df["latency"] <= cfg["Slo"])
            ]
            result["goodput_load_gen"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[
                (df["graph_id"] == graph_id)
                & (df["error"] == "/None")
                & (df["latency_fe"] <= cfg["Slo"])
            ]
            result["goodput_fe"] = round(len(df_filtered) / cfg["DurationSecs"])

            # result["goodput_per_rps"] = []
            # for i in range(0, len(df), rps):
            #     df_per_sec = df[i : i + rps]
            #     goodput_per_rps = len(
            #         df_per_sec[
            #             (df_per_sec["graph_id"] == graph_id)
            #             & (df_per_sec["error"] == "/None")
            #             & (df_per_sec["latency"] <= cfg["Slo"])
            #         ]
            #     )
            #     result["goodput_per_rps"].append(goodput_per_rps)

            rps_to_results.append(result)
            # plot_goodput_per_rps(result, f"fig_goodput_rps_{rps}_per_rps_{r}.png")

    plot_goodput_line(
        rps_to_results, f"{args.path}/fig_goodput_rps_line_{r}.png", args.mode
    )
    plot_goodput_bar(
        rps_to_results, f"{args.path}/fig_goodput_rps_bar_{r}.png", args.mode
    )
