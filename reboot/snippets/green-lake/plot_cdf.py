from typing import *

import matplotlib.pyplot as plt
import numpy as np
import pandas as pd

rps = 800
modes = ["masa", "fifo"]

results: Dict[str, List[int]] = {"masa": [], "fifo": []}

for mode in modes:
    file = f"r{rps}-{mode}.csv"
    df = pd.read_csv(file)

    df["latency"] /= 1_000
    latencies = list(df["latency"])
    results[mode] = latencies

fig = plt.figure(figsize=(10, 6))

for mode in modes:
    latencies = results[mode]
    latencies.sort()
    cdf = np.arange(1, len(latencies) + 1) / len(latencies)

    plt.plot(latencies, cdf, label=mode)

# plt.tight_layout()
plt.xlabel("Latency (ms)")
plt.ylabel("CDF")
plt.title("Latency CDF")
plt.legend()
plt.grid(True)
plt.savefig("fig_cdf.png")
plt.show()
