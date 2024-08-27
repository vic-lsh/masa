from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 50
modes = ["masa", "fifo"]

results_raw: Dict[str, List] = {}
sh_i1 = "say_hello_i1"
sh_i2 = "say_hello_i2"

for mode in modes:
    file = f"tmp_{mode}.log"
    ids_to_spans: Dict[int, List[Dict]] = {}

    with open(file) as f:
        lines = f.readlines()
        for line in lines:
            if sh_i1 in line or sh_i2 in line:
                splits = line.split()[-1].split(",")
                span: Dict[str, Any] = {
                    "span": splits[0],
                    "request_id": int(splits[1]),
                    "elapse": int(splits[2]),
                    "latency": int(splits[3]),
                }
                ids_to_spans.setdefault(span["request_id"], []).append(span)

    spans_queueing: List[Dict] = []
    for id in ids_to_spans:
        spans = ids_to_spans[id]
        if len(spans) != 2:
            continue
        span1 = spans[0]
        span2 = spans[1]
        if span1["span"] != sh_i1 or span2["span"] != sh_i2:
            continue
        span_queueing = {
            "request_id": id,
            "queueing": int(span2["latency"]) - int(span1["latency"]),
        }
        spans_queueing.append(span_queueing)

    results_raw[mode] = spans_queueing

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
    df.to_csv(f"r{rps}_{mode}_queueing_filtered.csv", index=False)
