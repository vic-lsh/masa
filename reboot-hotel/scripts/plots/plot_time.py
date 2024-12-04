import json
from typing import *

import pandas as pd
from plot_core import parse_args, plot_time_bar  # type: ignore

MS = 1e3

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
        print(f"Got empty data for good_total, rps: {rps}")
        result["good_total"] = 0
    else:
        result["good_total"] = sum(df_filtered["latency"])

    df_filtered = df[df["error"].str.contains("EarlyReturn")]
    if df_filtered.empty:
        print(f"Got empty data for err_svc_er_total, rps: {rps}")
        result["err_svc_er_total"] = 0
    else:
        result["err_svc_er_total"] = sum(df_filtered["latency"])

    df_filtered = df[df["error"] == "/ClientMiss"]
    if df_filtered.empty:
        print(f"Got empty data for err_client_miss_total, rps: {rps}")
        result["err_cl_miss_total"] = 0
    else:
        result["err_cl_miss_total"] = sum(df_filtered["latency"])

    rps_to_results.append(result)

plot_time_bar(
    rps_to_results,
    f"{args.path}/time/fig_time_total_rps_bar_{r}.png",
    f"{args.mode} total",
)
