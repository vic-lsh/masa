from typing import *

import pandas as pd

rps = [r for r in range(300, 901, 25)]
modes = ["masa", "fifo"]

results: Dict[str, List[int]] = {"rps": rps, "masa": [], "fifo": [], "relative": []}

for r in rps:
    for mode in modes:
        file = f"r{r}-{mode}.csv"
        df = pd.read_csv(file)

        latencies = df["latency"].quantile(0.99)
        results[mode].append(round(latencies))

    results["relative"].append(results["fifo"][-1] * 100 // results["masa"][-1])

df = pd.DataFrame(results)
df.to_csv("results.csv", index=False)
