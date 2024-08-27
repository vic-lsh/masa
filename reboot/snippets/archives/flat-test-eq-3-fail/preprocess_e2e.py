from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 800
modes = ["masa", "fifo"]

results_raw: Dict[str, List] = {}
for mode in modes:
    file = f"r{rps}_{mode}.csv"
    df = pd.read_csv(file)

    spans: List[Dict] = []
    for _, row in df.iterrows():
        id = int(row["request_id"])
        span = row["span"]
        slo = int(row["slo"])
        latency = int(row["latency"])
        spans.append({"request_id": id, "span": span, "slo": slo, "latency": latency})
    results_raw[mode] = spans

ids_to_modes: Dict[int, List[str]] = {}
for mode in modes:
    spans = results_raw[mode]
    for span in spans:
        id = span["request_id"]
        ids_to_modes.setdefault(id, []).append(mode)

common_ids = set(
    id for id, modes in ids_to_modes.items() if len(modes) == 2 and modes[0] != modes[1]
)

for mode in modes:
    spans_filtered = []
    spans = results_raw[mode]
    for span in spans:
        id = span["request_id"]
        if id in common_ids:
            spans_filtered.append(span)
    spans_filtered = sorted(spans_filtered, key=lambda x: x["request_id"])
    df = pd.DataFrame(spans_filtered)
    df.to_csv(f"r{rps}_{mode}_filtered.csv", index=False)
