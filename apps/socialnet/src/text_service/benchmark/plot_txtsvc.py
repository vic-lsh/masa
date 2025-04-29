import pandas as pd
import matplotlib.pyplot as plt

# --- 1. Load -------------------------------------------------------------
CSV_PATH = "/home/jiexiao/research/masa-internal/apps/socialnet/src/text_service/benchmark/textservice_bench.csv"
df = pd.read_csv(CSV_PATH)

# (optional) drop the first second where goodput/latency are 0
df2 = df[df["sec"] > 0]

# --- 2. Aggregate: one row per RPS --------------------------------------
summary = (
    df2.groupby("rps", as_index=False)
       .agg(goodput_mean=("goodput", "mean"),
            latency_mean=("avg_latency", "mean"))
)

# --- 3. Plot -------------------------------------------------------------
fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(10, 4))

# Goodput vs RPS
ax1.plot(summary["rps"], summary["goodput_mean"], marker="o")
ax1.set_title("Goodput vs RPS")
ax1.set_xlabel("Target RPS")
ax1.set_ylabel("Measured goodput (req/s)")
ax1.grid(True)

# Latency vs RPS
ax2.plot(summary["rps"], summary["latency_mean"], marker="s", color="tab:orange")
ax2.set_title("Latency vs RPS")
ax2.set_xlabel("Target RPS")
ax2.set_ylabel("Average latency (µs)")
ax2.grid(True)

fig.suptitle("Load-test summary (one point per RPS)")
fig.tight_layout()
plt.savefig("load_test_summary.png", dpi=300)
