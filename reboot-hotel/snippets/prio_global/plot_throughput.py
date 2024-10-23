import json
from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, plot_throughput  # type: ignore

cfg = json.load(open("gen_config.json"))

for r in range(cfg["Repeats"]):
    rps_to_results: List[Dict[str, Any]] = []

    for rps in cfg["Rps"]:
        file = f"r{rps}_{r}.csv"
        df = pd.read_csv(file)
        for graph_id in GRAPH_IDS:
            result = {}
            result["rps"] = rps
            result["graph_id"] = graph_id

            df_filtered = df[(df["graph_id"] == graph_id)]
            result["tput_all"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[
                (df["graph_id"] == graph_id)
                & (df["error"] == "/None")
                & (df["latency_fe"] <= cfg["Slo"])
            ]
            result["tput_good"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[
                (df["graph_id"] == graph_id)
                & (df["error"] == "/None")
                & (df["latency_fe"] > cfg["Slo"])
            ]
            result["tput_neutral"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[(df["graph_id"] == graph_id) & (df["error"] != "/None")]
            result["tput_error"] = round(len(df_filtered) / cfg["DurationSecs"])

            rps_to_results.append(result)

    plot_throughput(rps_to_results, f"fig_throughput_rps_{r}.png")
