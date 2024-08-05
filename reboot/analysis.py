import pandas as pd

files = {
    "fifo": "tmp_fifo.csv",
    "masa": "tmp_masa.csv",
}

for name, file in files.items():
    print(f"Results for {name}:")

    df = pd.read_csv(file)

    mean_value = df["latency"].mean()
    stdev_value = df["latency"].std()

    print(f"mean: {round(mean_value)}")
    print(f"std: {round(stdev_value)}")

    percentiles = [0.5, 0.9, 0.99]
    latencies = list(df["latency"].quantile(percentiles))
    for i, p in enumerate(percentiles):
        print(f"{int(p*100)}%: {round(latencies[i])}")
    
    print()
