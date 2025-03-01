import json
from typing import *

import pandas as pd
from plot_core import (  # type: ignore
    parse_args,
    plot_goodput_apis_bar,
    plot_goodput_bar,
)

args = parse_args()
cfg = json.load(open(args.gen_config))

r = 0
rps_to_results: List[Dict[str, Any]] = []

for rps in cfg["Rps"]:
    file = f"{args.path}/r{rps}_{r}.csv"
    df = pd.read_csv(file)

    result = {}
    result["rps"] = rps
    result["goodput_load_gen"] = 0

    for i in range(len(cfg["Apis"])):
        api = cfg["Apis"][i]
        slo = cfg["Slos"][i]
        df_filtered = df[
            (df["api"] == api) & (df["error"] == "/None") & (df["latency"] <= slo)
        ]
        result["goodput_load_gen"] += round(len(df_filtered) / cfg["DurationSecs"])

    rps_to_results.append(result)

plot_goodput_bar(
    cfg["Apis"],
    args.mode,
    rps_to_results,
    f"{args.path}/goodput/fig_goodput_rps_{r}.png",
)
if len(cfg["Apis"]) == 1:
    exit()

rps_to_results = []

for rps in cfg["Rps"]:
    file = f"{args.path}/r{rps}_{r}.csv"
    df = pd.read_csv(file)

    for i in range(len(cfg["Apis"])):
        result = {}
        result["rps"] = rps
        result["api"] = cfg["Apis"][i]

        api = cfg["Apis"][i]
        slo = cfg["Slos"][i]
        df_filtered = df[
            (df["api"] == api) & (df["error"] == "/None") & (df["latency"] <= slo)
        ]
        result["goodput_load_gen"] = round(len(df_filtered) / cfg["DurationSecs"])

        rps_to_results.append(result)

plot_goodput_apis_bar(
    cfg["Apis"],
    args.mode,
    rps_to_results,
    f"{args.path}/goodput/fig_goodput_apis_rps_{r}.png",
)
