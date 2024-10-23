import json
from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, parse_args, plot_error  # type: ignore

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

            df_filtered = df[(df["graph_id"] == graph_id) & (df["error"] != "/None")]
            result["error_all"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[(df["graph_id"] == graph_id) & (df["error"] == "/LoadGen")]
            result["error_load_gen"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[
                (df["graph_id"] == graph_id)
                & (df["error"] == "/search.Search/HandleNearby")
            ]
            result["error_search"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[
                (df["graph_id"] == graph_id)
                & (df["error"] == "/profile.Profile/HandleGetProfiles")
            ]
            result["error_profile"] = round(len(df_filtered) / cfg["DurationSecs"])

            rps_to_results.append(result)

    plot_error(rps_to_results, f"{args.path}/fig_error_rps_{r}.png", args.mode)
