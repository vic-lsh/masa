import pandas as pd
import matplotlib.pyplot as plt
import os
import re

FOLDER_PATH = "/home/jiexiao/research/masa-internal/apps/hotel/data/out/trace_01/0/fifo"

# Read all file that end with csv
csv_files = sorted([f for f in os.listdir(FOLDER_PATH) if f.endswith('.csv')])

# Regex to get rps with format "r{rps}_Search.csv"
rps_pattern = r"r(?P<rps>\d+)_Search\.csv"

data_to_plot = {}

for file in csv_files:
    match = re.search(rps_pattern, file)
    if match:
        rps_value = int(match.group('rps'))
        file_path = os.path.join(FOLDER_PATH, file)
        try:
            df = pd.read_csv(file_path)
            df = df[df['error'] == '/None']
            # df = df[df['error'] != '/ClientTimeout']

            # Calculate mean latencies
            mean_latencies = {
                'e2e': df['latency'].mean(),
                'search': {
                    'compute': df['child_search_compute_latency'].mean(),
                    'io': df['child_search_io_latency'].mean(),
                    'queue': df['child_search_queue_latency'].mean(),
                },
                'reserve': {
                    'compute': df['child_reserve_compute_latency'].mean(),
                    'io': df['child_reserve_io_latency'].mean(),
                    'queue': df['child_reserve_queue_latency'].mean(),
                },
                'profile': {
                    'compute': df['child_profile_compute_latency'].mean(),
                    'io': df['child_profile_io_latency'].mean(),
                    'queue': df['child_profile_queue_latency'].mean(),
                }
            }
            data_to_plot[rps_value] = mean_latencies

        except Exception as e:
            print(f"Error reading {file}: {e}")
            continue

# Sort the data by RPS value
sorted_rps = sorted(data_to_plot.keys())

# Define plotting parameters
bar_width = 0.2
colors = {'compute': "#eaff64", 'io': "#3182bd", 'queue': "#11EAB4", 'e2e': '#969696'}
latency_components = ['compute', 'io', 'queue']
services = ['Overall E2E', 'Search', 'Profile', 'Reserve']

# --- Plot 1: Detailed view of the first RPS ---
if sorted_rps:
    first_rps = sorted_rps[0]
    data_for_first_rps = data_to_plot[first_rps]
    detailed_bar_width = 0.6
    
    plt.style.use('seaborn-v0_8-whitegrid')
    fig1, ax1 = plt.subplots(figsize=(12, 8))
    x_positions = [0, 1.2, 2.4, 3.6]
    
    # Overall E2E bar
    e2e_latency = data_for_first_rps['e2e']
    ax1.bar(x_positions[0], e2e_latency, width=detailed_bar_width, color=colors['e2e'], label=services[0])
    ax1.text(x_positions[0], e2e_latency + 50, f"{e2e_latency:.1f}", ha='center')
    
    # Search stacked bar
    search_total = sum(data_for_first_rps['search'][c] for c in latency_components)
    bottom = 0
    for i, component in enumerate(latency_components):
        value = data_for_first_rps['search'][component]
        ax1.bar(x_positions[1], value, bottom=bottom, width=detailed_bar_width, color=colors[component])
        bottom += value
    ax1.text(x_positions[1], search_total + 50, f"{search_total:.1f}", ha='center')

    # Profile stacked bar
    profile_total = sum(data_for_first_rps['profile'][c] for c in latency_components)
    bottom = 0
    for i, component in enumerate(latency_components):
        value = data_for_first_rps['profile'][component]
        ax1.bar(x_positions[2], value, bottom=bottom, width=detailed_bar_width, color=colors[component])
        bottom += value
    ax1.text(x_positions[2], profile_total + 50, f"{profile_total:.1f}", ha='center')

    # Reserve stacked bar
    reserve_total = sum(data_for_first_rps['reserve'][c] for c in latency_components)
    bottom = 0
    for i, component in enumerate(latency_components):
        value = data_for_first_rps['reserve'][component]
        ax1.bar(x_positions[3], value, bottom=bottom, width=detailed_bar_width, color=colors[component])
        bottom += value
    ax1.text(x_positions[3], reserve_total + 50, f"{reserve_total:.1f}", ha='center')

    # Create a custom legend for the components
    handles = []
    labels = []
    handles.append(plt.Rectangle((0, 0), 1, 1, fc=colors['e2e']))
    labels.append('Overall E2E Latency')
    for component in latency_components:
        handles.append(plt.Rectangle((0, 0), 1, 1, fc=colors[component]))
        labels.append(f'{component.capitalize()} Latency')

    ax1.legend(handles, labels)
    ax1.set_ylabel('Latency (microseconds)', fontsize=12)
    ax1.set_title(f'Detailed Service Latency Breakdown at {first_rps} RPS', fontsize=14, fontweight='bold')
    ax1.set_xticks(x_positions)
    ax1.set_xticklabels(services, rotation=0)
    ax1.set_ylim(top=ax1.get_ylim()[1] * 1.1) # Add some space on top for the labels

    plt.tight_layout()
    plt.savefig('latency_breakdown_first_rps.png')

# --- Plot 2: Summary view of all RPS values ---
plt.style.use('seaborn-v0_8-whitegrid')
fig2, ax2 = plt.subplots(figsize=(15, 8))
x_base_positions = range(len(sorted_rps))

for i, rps_val in enumerate(sorted_rps):
    # Overall E2E bar
    e2e_pos = x_base_positions[i] - 1.5 * bar_width
    ax2.bar(e2e_pos, data_to_plot[rps_val]['e2e'], width=bar_width, color=colors['e2e'])

    # Search stacked bar
    search_pos = x_base_positions[i] - 0.5 * bar_width
    bottom = 0
    for component in latency_components:
        ax2.bar(search_pos, data_to_plot[rps_val]['search'][component], bottom=bottom, width=bar_width, color=colors[component])
        bottom += data_to_plot[rps_val]['search'][component]

    # Profile stacked bar
    profile_pos = x_base_positions[i] + 0.5 * bar_width
    bottom = 0
    for component in latency_components:
        ax2.bar(profile_pos, data_to_plot[rps_val]['profile'][component], bottom=bottom, width=bar_width, color=colors[component])
        bottom += data_to_plot[rps_val]['profile'][component]

    # Reserve stacked bar
    reserve_pos = x_base_positions[i] + 1.5 * bar_width
    bottom = 0
    for component in latency_components:
        ax2.bar(reserve_pos, data_to_plot[rps_val]['reserve'][component], bottom=bottom, width=bar_width, color=colors[component])
        bottom += data_to_plot[rps_val]['reserve'][component]

# Create a custom legend for the components
handles = []
labels = []
handles.append(plt.Rectangle((0, 0), 1, 1, fc=colors['e2e']))
labels.append('Overall E2E Latency')
for component in latency_components:
    handles.append(plt.Rectangle((0, 0), 1, 1, fc=colors[component]))
    labels.append(f'{component.capitalize()} Latency')

ax2.legend(handles, labels)
ax2.set_ylabel('Latency (microseconds)', fontsize=12)
ax2.set_xlabel('RPS', fontsize=12)
ax2.set_title('Service Latency Breakdown by RPS', fontsize=14, fontweight='bold')
ax2.set_xticks(x_base_positions)
ax2.set_xticklabels([f'{rps} RPS' for rps in sorted_rps])

plt.tight_layout()
plt.savefig('latency_breakdown_all_rps.png')

import pandas as pd
import matplotlib.pyplot as plt
import matplotlib.dates as mdates

# ----- Config -----
DOCKER_STATS_PATH = "/home/jiexiao/research/masa-internal/apps/hotel/data/container_stats.csv"  
SERVICE_CONTAINER = {
    "Search":  "local-search-service-1",
    "Profile": "local-profile-service-1",
    "Reserve": "local-reservation-service-1",
}
RESAMPLE_RULE = "1S"    # resample cadence (1 second)
ROLLING_WINDOW = 5       # smoothing window in samples; set 0 to disable

# ----- Load & clean -----
df = pd.read_csv(DOCKER_STATS_PATH)

# Parse CPU% like "35.97%" -> 35.97
df["cpu_perc"] = pd.to_numeric(
    df["cpu_perc"].astype(str).str.strip().str.rstrip("%"),
    errors="coerce"
)

# Convert ts_ms (epoch milliseconds) to Python datetime
df["ts"] = pd.to_datetime(pd.to_numeric(df["ts_ms"], errors="coerce"), unit="ms")

# Keep rows we can use
df = df.dropna(subset=["ts", "cpu_perc", "container"])

# ----- Filter to the three services -----
containers_of_interest = set(SERVICE_CONTAINER.values())
df = df[df["container"].isin(containers_of_interest)].copy()

if df.empty:
    raise SystemExit("No matching rows for the specified service containers. "
                     "Check the names in SERVICE_CONTAINER against your CSV.")

# ----- Pivot to time series (columns per container) -----
# If multiple samples share the same ts for a container, average them first
pivot = (df.groupby(["ts", "container"])["cpu_perc"]
           .mean()
           .unstack("container"))

# Resample to a uniform time grid (helps plotting)
pivot = pivot.resample(RESAMPLE_RULE).mean()

# Optional smoothing to reduce jitter
if ROLLING_WINDOW and ROLLING_WINDOW > 1:
    pivot = pivot.rolling(window=ROLLING_WINDOW, min_periods=1).median()

# Rename columns to friendly service names
rename_map = {v: k for k, v in SERVICE_CONTAINER.items()}
pivot = pivot.rename(columns=rename_map)

# ----- Plot -----
plt.figure(figsize=(14, 6))
for col in ["Search", "Profile", "Reserve"]:
    if col in pivot.columns:
        plt.plot(pivot.index, pivot[col], label=col)

plt.title("CPU Utilization Over Time (docker stats)")
plt.xlabel("Time")
plt.ylabel("CPU Utilization (%)")
plt.gca().xaxis.set_major_formatter(mdates.DateFormatter("%H:%M:%S"))
plt.gcf().autofmt_xdate()
plt.legend(loc="upper left")
plt.tight_layout()
plt.savefig("cpu_utilization_search_profile_reserve.png", dpi=200)
# plt.show()  # uncomment if you want an interactive window
