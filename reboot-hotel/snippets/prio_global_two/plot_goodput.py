import json
from typing import *

import pandas as pd
from plot_core import (  # type: ignore
    GRAPH_IDS,
    MODES,
    plot_cdf,
    plot_goodput,
    plot_goodput_per_rps,
    plot_pdf,
)

cfg = json.load(open("gen_config.json"))

for mode in MODES:
    # [TODO] Also plot tail latency.
    for r in range(cfg["Repeats"]):
        rps_to_results: List[Dict[str, Any]] = []

        for rps in cfg["Rps"]:
            file = f"r{rps}_{r}.csv"
            df = pd.read_csv(file)
            for graph_id in GRAPH_IDS:
                result = {}
                result["rps"] = rps
                result["graph_id"] = graph_id

                df_filtered = df[
                    (df["graph_id"] == graph_id)
                    & (df["latency"] <= cfg["Slo"])
                    & (df["error"] == False)
                ]
                result["goodput"] = round(len(df_filtered) / cfg["DurationSecs"])

                result["goodput_per_rps"] = []
                for i in range(0, len(df), rps):
                    df_per_sec = df[i : i + rps]
                    goodput_per_rps = len(
                        df_per_sec[
                            (df_per_sec["graph_id"] == graph_id)
                            & (df_per_sec["latency"] <= cfg["Slo"])
                            & (df_per_sec["error"] == False)
                        ]
                    )
                    result["goodput_per_rps"].append(goodput_per_rps)

                rps_to_results.append(result)
                plot_goodput_per_rps(result, f"goodput_rps_{rps}_per_sec_{r}.png")

        plot_goodput(rps_to_results, f"goodput_rps_{r}.png")
