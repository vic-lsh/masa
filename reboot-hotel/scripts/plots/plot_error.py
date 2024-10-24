import json
from typing import *

import pandas as pd
from plot_core import (  # type: ignore
    GRAPH_IDS,
    parse_args,
    plot_error_bar,
    plot_error_line,
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

            df_filtered = df[(df["graph_id"] == graph_id) & (df["error"] == "/LGMiss")]
            result["error_lg_miss"] = round(len(df_filtered) / cfg["DurationSecs"])

            df_filtered = df[
                (df["graph_id"] == graph_id) & (df["error"] == "/LGTimeout")
            ]
            result["error_lg_timeout"] = round(len(df_filtered) / cfg["DurationSecs"])

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

    plot_error_line(
        rps_to_results, f"{args.path}/fig_error_rps_line_{r}.png", args.mode
    )
    plot_error_bar(rps_to_results, f"{args.path}/fig_error_rps_bar_{r}.png", args.mode)
