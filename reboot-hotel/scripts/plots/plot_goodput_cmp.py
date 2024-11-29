import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_goodput_cmp_bar  # type: ignore

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
        result["mode"] = mode
        result["goodput"] = 0

        for i in range(len(cfg["Apis"])):
            api = cfg["Apis"][i]
            slo = cfg["Slos"][i]
            df_filtered = df[
                (df["api"] == api) & (df["error"] == "/None") & (df["latency"] <= slo)
            ]
            result["goodput"] += round(len(df_filtered) / cfg["DurationSecs"])

        rps_to_results.append(result)

modes_str = "_".join(args.modes)
plot_goodput_cmp_bar(
    args.modes,
    "total",
    rps_to_results,
    f"{args.path}/goodput/fig_goodput_total_rps_cmp_{modes_str}_{r}.png",
)

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
            result["mode"] = mode
            result["api"] = api
            result["goodput"] = 0

            idx = cfg["Apis"].index(api)
            api = cfg["Apis"][idx]
            slo = cfg["Slos"][idx]
            df_filtered = df[
                (df["api"] == api) & (df["error"] == "/None") & (df["latency"] <= slo)
            ]
            result["goodput"] = round(len(df_filtered) / cfg["DurationSecs"])

            rps_to_results.append(result)

    api_lower = api.lower()
    plot_goodput_cmp_bar(
        args.modes,
        api_lower,
        rps_to_results,
        f"{args.path}/goodput/fig_goodput_{api_lower}_rps_cmp_{api_lower}_{modes_str}_{r}.png",
    )
