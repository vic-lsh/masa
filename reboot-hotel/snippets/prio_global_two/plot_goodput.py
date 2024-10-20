import json
from typing import *

import pandas as pd
from plot_core import GRAPH_IDS, MODES, plot_cdf, plot_goodput, plot_pdf  # type: ignore

cfg = json.load(open("gen_config.json"))

for mode in MODES:
    # [TODO] Also plot tail latency.
    # e2e_results: List[Tuple[str, List[int]]] = []
    for r in range(cfg["Repeats"]):
        goodput_results: List[Dict[str, Any]] = []

        for rps in cfg["Rps"]:
            file = f"r{rps}_{r}.csv"
            df = pd.read_csv(file)
            for graph_id in GRAPH_IDS:
                df_filtered = df[
                    (df["graph_id"] == graph_id)
                    & (df["latency"] <= cfg["Slo"])
                    & (df["error"] == False)
                ]
                result = {}
                result["rps"] = rps
                result["graph_id"] = graph_id
                result["goodput"] = round(len(df_filtered) / cfg["DurationSecs"])
                goodput_results.append(result)

        plot_goodput(goodput_results, f"goodput_rps_{r}.png")
