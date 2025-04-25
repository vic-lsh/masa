import pandas as pd
import matplotlib.pyplot as plt

# Load CSV
df = pd.read_csv("/home/jiexiao/research/masa-internal/apps/socialnet/src/text_service/benchmark/textservice_bench.csv")

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(10, 4), sharex=True)

for rps, grp in df.groupby("rps"):
    # sec is already 0..duration-1 for each phase
    ax1.plot(grp["sec"], grp["goodput"], marker="o", label=f"{rps} RPS")
    ax2.plot(grp["sec"], grp["avg_latency"], marker="s", label=f"{rps} RPS")

ax1.set_title("Goodput")
ax1.set_xlabel("second")
ax1.set_ylabel("requests / s")
ax1.grid(True)
ax1.legend(frameon=False)

ax2.set_title("Average latency")
ax2.set_xlabel("second")
ax2.set_ylabel("µs")
ax2.grid(True)
ax2.legend(frameon=False)

fig.suptitle("Load-test phases by RPS")
fig.tight_layout()
plt.savefig("load_test.png", dpi=300)