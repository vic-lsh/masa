import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_tail_bar  # type: ignore

MS = 1e3
PCTLS = [50, 90, 95, 99]

args = parse_args()
cfg = json.load(open(args.gen_config))

r = 0
rps_to_results: List[Dict[str, Any]] = []

for rps in cfg["Rps"]:
    file = f"{args.path}/r{rps}_{r}.csv"
    df = pd.read_csv(file)

    result = {}
    result["rps"] = rps

    df_filtered = df[df["error"] == "/None"]
    if df_filtered.empty:
        print(f"Expect non-empty data, rps: {rps}")
        result["mean"] = 0
        for p in PCTLS:
            result[f"p{p}"] = 0
    else:
        result["mean"] = round(df_filtered["latency"].mean() / MS)
        for p in PCTLS:
            result[f"p{p}"] = round(df_filtered["latency"].quantile(p / 100) / MS)

    rps_to_results.append(result)

plot_tail_bar(
    rps_to_results,
    f"{args.path}/tail/fig_tail_total_rps_bar_{r}.png",
    f"{args.mode} total",
)

if len(cfg["Apis"]) == 1:
    exit()

for api in cfg["Apis"]:
    rps_to_results = []

    for rps in cfg["Rps"]:
        file = f"{args.path}/r{rps}_{r}.csv"
        df = pd.read_csv(file)

        result = {}
        result["rps"] = rps

        df_filtered = df[(df["error"] == "/None") & (df["api"] == api)]
        if df_filtered.empty:
            print(f"Expect non-empty data, rps: {rps}")
            result["mean"] = 0
            for p in PCTLS:
                result[f"p{p}"] = 0
        else:
            result["mean"] = round(df_filtered["latency"].mean() / MS)
            for p in PCTLS:
                result[f"p{p}"] = round(df_filtered["latency"].quantile(p / 100) / MS)

        rps_to_results.append(result)

    api_lower = api.lower()
    plot_tail_bar(
        rps_to_results,
        f"{args.path}/tail/fig_tail_{api_lower}_rps_{r}.png",
        f"{args.mode} {api_lower}",
    )
