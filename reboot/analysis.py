import pandas as pd

# Load the data from the CSV file
df = pd.read_csv("tmp.csv")

# Calculate the mean and standard deviation of the specified column
mean_value = df["latency"].mean()
stdev_value = df["latency"].std()

print(f"mean: {round(mean_value)}")
print(f"std: {round(stdev_value)}")

percentiles = [0.5, 0.9, 0.99]
latencies = list(df["latency"].quantile(percentiles))
for i, p in enumerate(percentiles):
    print(f"{int(p*100)}%: {round(latencies[i])}")
