from collections import defaultdict
from typing import *

import pandas as pd

rps = 50
modes = ["queue_edf", "queue_fifo_infra", "queue_fifo"]

results_all: Dict[str, List] = {}
results_infra: Dict[str, List] = {}
results_req: Dict[str, List] = {}
for mode in modes:
    file = f"tmp_{mode}.log"
    spans_all: List[Dict] = []
    spans_infra: List[Dict] = []
    spans_req: List[Dict] = []
    task_timestamps: Dict[str, int] = defaultdict(int)
    with open(file) as f:
        lines = f.readlines()
        for line in lines:
            contents = line.split(",")

            def parse_content(content: str) -> Dict[str, Any]:
                data: Dict[str, Any] = {}
                for content in contents:
                    if ":" in content:
                        key, value = content.split(":")
                        key = key.strip()
                        value = value.strip()
                        if key == "now":
                            data[key] = int(value)
                        elif key == "deadline":
                            data[key] = int(value)
                        else:
                            data[key] = value
                return data

            if "Push runnable to queue" in line:
                data = parse_content(contents[1])
                task = data["task"]
                timestamp = data["now"]
                assert task_timestamps[task] == 0
                task_timestamps[task] = timestamp
            elif "Pop runnable from queue" in line:
                data = parse_content(contents[1])
                task = data["task"]
                timestamp = data["now"]
                deadline = data["deadline"]
                assert task_timestamps[task] != 0
                span = {
                    "task": task,
                    "queueing": timestamp - task_timestamps[task],
                }
                spans_all.append(span)
                if deadline == 0:
                    spans_infra.append(span)
                else:
                    spans_req.append(span)
                task_timestamps[task] = 0
    results_all[mode] = spans_all
    results_infra[mode] = spans_infra
    results_req[mode] = spans_req

for mode in modes:
    spans_all = results_all[mode]
    df = pd.DataFrame(spans_all)
    df.to_csv(f"r{rps}_{mode}_all_queueing.csv", index=False)

    spans_all = results_infra[mode]
    df = pd.DataFrame(spans_all)
    df.to_csv(f"r{rps}_{mode}_infra_queueing.csv", index=False)

    spans_all = results_req[mode]
    df = pd.DataFrame(spans_all)
    df.to_csv(f"r{rps}_{mode}_req_queueing.csv", index=False)

"""
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
"""
