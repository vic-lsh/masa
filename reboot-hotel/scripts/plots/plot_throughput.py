import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_throughput_bar  # type: ignore

args = parse_args()
cfg = json.load(open(args.gen_config))

r = 0
rps_to_results: List[Dict[str, Any]] = []

for rps in cfg["Rps"]:
    file = f"{args.path}/r{rps}_{r}.csv"
    df = pd.read_csv(file)

    result = {}
    result["rps"] = rps
    result["goodput"] = 0

    for i in range(len(cfg["Apis"])):
        api = cfg["Apis"][i]
        slo = cfg["Slos"][i]
        df_filtered = df[
            (df["api"] == api) & (df["error"] == "/None") & (df["latency"] <= slo)
        ]
        result["goodput"] += round(len(df_filtered) / cfg["DurationSecs"])

    df_filtered = df[df["error"] == "/LGMiss"]
    result["lg_miss"] = round(len(df_filtered) / cfg["DurationSecs"])

    df_filtered = df[df["error"] == "/LGTimeout"]
    result["lg_timeout"] = round(len(df_filtered) / cfg["DurationSecs"])

    df_filtered = df[df["error"].str.contains("EarlyReturn")]
    result["early_return"] = round(len(df_filtered) / cfg["DurationSecs"])

    errors = ["/None", "/LGMiss", "/LGTimeout"]
    df_filtered = df[
        (~df["error"].isin(errors)) & (~df["error"].str.contains("EarlyReturn"))
    ]
    result["unknown"] = round(len(df_filtered) / cfg["DurationSecs"])

    rps_to_results.append(result)

plot_throughput_bar(
    args.mode,
    rps_to_results,
    f"{args.path}/fig_throughput_total_rps_{r}.png",
)

if len(cfg["Apis"]) == 1:
    exit()

for i, api in enumerate(cfg["Apis"]):
    rps_to_results = []

    for rps in cfg["Rps"]:
        file = f"{args.path}/r{rps}_{r}.csv"
        df = pd.read_csv(file)

        result = {}
        result["rps"] = rps

        slo = cfg["Slos"][i]
        df_filtered = df[
            (df["api"] == api) & (df["error"] == "/None") & (df["latency"] <= slo)
        ]
        result["goodput"] = round(len(df_filtered) / cfg["DurationSecs"])

        df_filtered = df[df["error"] == "/LGMiss"]
        result["lg_miss"] = round(len(df_filtered) / cfg["DurationSecs"])

        df_filtered = df[df["error"] == "/LGTimeout"]
        result["lg_timeout"] = round(len(df_filtered) / cfg["DurationSecs"])

        df_filtered = df[
            (df["error"].str.contains("EarlyReturn")) & (df["error"].str.contains(api))
        ]
        result["early_return"] = round(len(df_filtered) / cfg["DurationSecs"])

        errors = ["/None", "/LGMiss", "/LGTimeout"]
        df_filtered = df[
            (~df["error"].isin(errors)) & (~df["error"].str.contains("EarlyReturn"))
        ]
        result["unknown"] = round(len(df_filtered) / cfg["DurationSecs"])

        rps_to_results.append(result)

    api_lower = api.lower()
    plot_throughput_bar(
        args.mode,
        rps_to_results,
        f"{args.path}/fig_throughput_{api_lower}_rps_{r}.png",
    )
