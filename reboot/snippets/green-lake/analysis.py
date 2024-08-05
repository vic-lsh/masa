import pandas as pd

# rps = [100, 200]
# modes = ["masa", "fifo"]

rps = [700, 750, 800, 850, 900]
modes = ["masa", "fifo"]

print(rps)

for mode in modes:
    print(f"Results for {mode}:")
    for r in rps:
        file = f"r{r}-{mode}.csv"
        df = pd.read_csv(file)

        percentiles = [0.99]
        latencies = list(df["latency"].quantile(percentiles))
        for i, p in enumerate(percentiles):
            print(round(latencies[i]))

    print()
