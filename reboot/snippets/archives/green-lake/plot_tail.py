import matplotlib.pyplot as plt
import pandas as pd

plt.rcParams["font.family"] = "Roboto"

fontsize = 20
plt.rcParams.update(
    {
        "font.size": fontsize,
        "axes.labelsize": fontsize,
        "axes.titlesize": fontsize,
        "xtick.labelsize": fontsize,
        "ytick.labelsize": fontsize,
        "legend.fontsize": fontsize,
    }
)

# Load the compiled results
df = pd.read_csv("results.csv")
df["masa"] /= 1_000
df["fifo"] /= 1_000

# Create a figure and a set of subplots
fig, ax1 = plt.subplots(figsize=(10, 6))

# Plot latencies for 'masa' and 'fifo'
ax1.plot(df["rps"], df["masa"], color="green", label="Masa", marker="o")
ax1.plot(df["rps"], df["fifo"], color="red", label="FIFO", marker="^")

# Label the x-axis and the primary y-axis
ax1.set_xlabel("RPS")
ax1.set_ylabel("P99 Latency (ms)")
ax1.set_ylim(0, 100)

# Create a second y-axis for the relative latency
ax2 = ax1.twinx()
ax2.plot(
    df["rps"],
    df["relative"],
    color="blue",
    label="Relative",
    marker="d",
    linestyle="--",
)

# Label the secondary y-axis
ax2.set_ylabel("Relative (%)")
ax2.set_ylim(100, 150)

# Add a legend to the plot
lines, labels = ax1.get_legend_handles_labels()
lines2, labels2 = ax2.get_legend_handles_labels()
ax1.legend(lines + lines2, labels + labels2, loc="upper left")

# Save the plot as a PNG file
plt.tight_layout()
plt.title("Tail Latency Comparison")
plt.savefig("fig_tail.png")
plt.show()
