from typing import *

import pandas as pd

percentiles = [0.1, 0.25, 0.5, 0.75, 0.9, 0.99]
# rps = [r for r in range(300, 901, 25)]
rps = [200, 1600]
modes = ["masa", "fifo"]

for pctl in percentiles:
    results: Dict[str, List[int]] = {"rps": rps, "masa": [], "fifo": [], "relative": []}

    for r in rps:
        for mode in modes:
            file = f"r{r}-{mode}.csv"
            df = pd.read_csv(file)

            latencies = df["latency"].quantile(pctl)
            results[mode].append(round(latencies))

        results["relative"].append(results["fifo"][-1] * 100 // results["masa"][-1])

    df = pd.DataFrame(results)
    df.to_csv(f"fig_tail_{pctl}.csv", index=False)
