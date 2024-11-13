import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_tail_bar  # type: ignore

args = parse_args()
cfg = json.load(open(f"{args.path}/gen_config.json"))
ms = 1e3

for r in range(cfg["Repeats"]):
    rps_to_results: List[Dict[str, Any]] = []

    for rps in cfg["Rps"]:
        file = f"{args.path}/r{rps}_{r}.csv"
        df = pd.read_csv(file)

        result = {}
        result["rps"] = rps
        df_filtered = df[df["error"].isin(["/None", "/LGMiss"])]
        if df_filtered.empty:
            print(f"Empty data for rps: {rps}")
            result["mean"] = 0
            result["p90"] = 0
            result["p95"] = 0
            result["p99"] = 0
        else:
            result["mean"] = round(df_filtered["latency"].mean() / ms)
            result["p90"] = round(df_filtered["latency"].quantile(0.9) / ms)
            result["p95"] = round(df_filtered["latency"].quantile(0.95) / ms)
            result["p99"] = round(df_filtered["latency"].quantile(0.99) / ms)

        rps_to_results.append(result)

    plot_tail_bar(
        rps_to_results, f"{args.path}/fig_tail_rps_bar_{r}.png", f"{args.mode} all"
    )

    for i in range(len(cfg["Apis"])):
        rps_to_results = []
        api = cfg["Apis"][i]

        for rps in cfg["Rps"]:
            file = f"{args.path}/r{rps}_{r}.csv"
            df = pd.read_csv(file)

            result = {}
            result["rps"] = rps
            df_filtered = df[
                (df["error"].isin(["/None", "/LGMiss"])) & (df["api"] == api)
            ]
            if df_filtered.empty:
                print(f"Empty data for rps: {rps}")
                result["mean"] = 0
                result["p90"] = 0
                result["p95"] = 0
                result["p99"] = 0
            else:
                result["mean"] = round(df_filtered["latency"].mean() / ms)
                result["p90"] = round(df_filtered["latency"].quantile(0.9) / ms)
                result["p95"] = round(df_filtered["latency"].quantile(0.95) / ms)
                result["p99"] = round(df_filtered["latency"].quantile(0.99) / ms)

            rps_to_results.append(result)

        api_lower = api.lower()
        plot_tail_bar(
            rps_to_results,
            f"{args.path}/fig_tail_{api_lower}_rps_bar_{r}.png",
            f"{args.mode} {api_lower}",
        )
