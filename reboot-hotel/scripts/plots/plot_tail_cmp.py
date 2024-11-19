import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_tail_cmp_bar  # type: ignore

MS = 1e3
PCTLS = [50, 90, 95, 99]

args = parse_args()

r = 0
rps_to_results: List[Dict[str, Any]] = []

for mode in args.modes:
    path = f"{args.snippets}/{mode}"
    cfg = json.load(open(args.gen_config))

    for rps in cfg["Rps"]:
        file = f"{path}/{args.data}/r{rps}_{r}.csv"
        df = pd.read_csv(file)

        result = {}
        result["rps"] = rps

        df_filtered = df[df["error"] == "/None"]
        if df_filtered.empty:
            print(f"Expect non-empty data, mode: {mode}, rps: {rps}")
            result[f"mean_{mode}"] = 0
            for p in PCTLS:
                result[f"p{p}_{mode}"] = 0
        else:
            result[f"mean_{mode}"] = round(df_filtered["latency"].mean() / MS)
            for p in PCTLS:
                result[f"p{p}_{mode}"] = round(
                    df_filtered["latency"].quantile(p / 100) / MS
                )

        rps_to_results.append(result)

modes_str = "_".join(args.modes)
for tail in ["mean", "p50", "p90", "p95", "p99"]:
    plot_tail_cmp_bar(
        args.modes,
        tail,
        rps_to_results,
        f"{args.path}/fig_{tail}_total_rps_cmp_{modes_str}_{r}.png",
    )

if len(cfg["Apis"]) == 1:
    exit()

for api in cfg["Apis"]:
    rps_to_results = []

    for mode in args.modes:
        path = f"{args.snippets}/{mode}"
        cfg = json.load(open(args.gen_config))

        for rps in cfg["Rps"]:
            file = f"{path}/{args.data}/r{rps}_{r}.csv"
            df = pd.read_csv(file)

            result = {}
            result["rps"] = rps

            df_filtered = df[(df["error"] == "/None") & (df["api"] == api)]
            if df_filtered.empty:
                print(f"Expect non-empty data, mode: {mode}, rps: {rps}")
                result[f"mean_{mode}"] = 0
                for p in PCTLS:
                    result[f"p{p}_{mode}"] = 0
            else:
                result[f"mean_{mode}"] = round(df_filtered["latency"].mean() / MS)
                for p in PCTLS:
                    result[f"p{p}_{mode}"] = round(
                        df_filtered["latency"].quantile(p / 100) / MS
                    )

            rps_to_results.append(result)

    api_lower = api.lower()
    modes_str = "_".join(args.modes)
    for tail in ["mean", "p50", "p90", "p95", "p99"]:
        plot_tail_cmp_bar(
            args.modes,
            tail,
            rps_to_results,
            f"{args.path}/fig_{tail}_{api_lower}_rps_cmp_{modes_str}_{r}.png",
        )
