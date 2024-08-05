import pandas as pd

rps = [100, 200]
modes = ["fifo", "masa"]

for mode in modes:
    print(f"Results for {mode}:")
    for r in rps:
        print("rps:", r)

        file = f"r{r}-{mode}.csv"
        df = pd.read_csv(file)

        # mean_value = df["latency"].mean()
        # stdev_value = df["latency"].std()
        # print(f"mean: {round(mean_value)}")
        # print(f"std: {round(stdev_value)}")

        percentiles = [0.99]
        latencies = list(df["latency"].quantile(percentiles))
        for i, p in enumerate(percentiles):
            print(f"{int(p*100)}%: {round(latencies[i])}")

    print()
