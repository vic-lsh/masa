import os
import re
import pandas as pd
import numpy as np
import matplotlib.pyplot as plt
import seaborn as sns

def parse_latency(value_str):
    try:
        if 'ms' in value_str:
            return float(value_str.replace('ms', '')) * 1000
        elif 'µs' in value_str:
            return float(value_str.replace('µs', ''))
        elif 's' in value_str:
            return float(value_str.replace('s', '')) * 1_000_000
    except (ValueError, TypeError):
        return np.nan
    return np.nan

def process_log(root_dir):
    all_data = []
    log_pattern = re.compile(r"\[(\w+)\] Latency: ([\d\.]+[µms]+s)")

    for dirpath, dirnames, filenames in os.walk(root_dir):
        for filename in filenames:
            if filename.endswith(".log"):
                try:
                    parts = dirpath.split(os.sep)
                    discipline = parts[-1]
                    rps_str = parts[-2]
                    rps = int(rps_str.replace('rps', ''))
                    service = filename.replace('_latencies.log', '')

                    full_path = os.path.join(dirpath, filename)
                    with open(full_path, 'r') as f:
                        for line in f:
                            match = log_pattern.match(line)
                            if match:
                                metric_type, value_str = match.groups()
                                latency_us = parse_latency(value_str)
                                if not np.isnan(latency_us):
                                    all_data.append({
                                        'rps': rps,
                                        'discipline': discipline,
                                        'service': service,
                                        'metric': metric_type,
                                        'latency_us': latency_us
                                    })
                except (IndexError, ValueError) as e:
                    print(f"Skipping malformed path or filename: {dirpath}/{filename} - {e}")

    return pd.DataFrame(all_data)

def plot_latency(df, rps_level, fpath):
    print(f"file path is {fpath}")
    rps_df = df[df['rps'] == rps_level].copy()
    if rps_df.empty:
        print(f"No data found for RPS {rps_level}")
        return
    
    stats = rps_df.groupby(['discipline', 'service', 'metric'])['latency_us'].agg(
        average=np.mean,
        p99=lambda x: x.quantile(0.99)
    ).reset_index()
    
    for stat_name in ['average', 'p99']:
        plt.figure(figsize=(12, 7))
        sns.set_style("whitegrid")
        
        plot_data = stats[stats['metric'] == 'Response']
        if not plot_data.empty:
            ax = sns.barplot(data=plot_data, x='service', y=stat_name, hue='discipline')
            ax.set_title(f'{stat_name.capitalize()} Response Latency at {rps_level} RPS', fontsize=16)
            ax.set_ylabel('Latency (µs) - Log Scale', fontsize=12)
            ax.set_xlabel('Service', fontsize=12)
            ax.set_yscale('log') # Use a log scale for better visibility of wide-ranging values
            plt.xticks(rotation=15, ha="right")
            plt.tight_layout()
            save_path = os.path.join(fpath, f'rps{rps_level}_response_{stat_name}.png')
            plt.savefig(save_path)

    for stat_name in ['average', 'p99']:
        plt.figure(figsize=(12, 7))
        sns.set_style("whitegrid")

        plot_data = stats[stats['metric'] == 'Queue']
        if not plot_data.empty:
            ax = sns.barplot(data=plot_data, x='service', y=stat_name, hue='discipline')
            ax.set_title(f'{stat_name.capitalize()} Queue Latency at {rps_level} RPS', fontsize=16)
            ax.set_ylabel('Latency (µs)', fontsize=12)
            ax.set_xlabel('Service', fontsize=12)
            # Log scale is often not needed for queue latency, but can be enabled if values vary greatly
            # ax.set_yscale('log')
            plt.xticks(rotation=15, ha="right")
            plt.tight_layout()
            save_path = os.path.join(fpath, f'rps{rps_level}_queue_{stat_name}.png')
            plt.savefig(save_path)
            
if __name__ == '__main__':
    data_directory = 'lat_out'
    
    print("Processing log files...")
    df = process_log(data_directory)

    print(df)
    if not df.empty:
        print("Data processed successfully.")
        for rps in sorted(df['rps'].unique()):
            print(f"\n--- Generating plots for {rps} RPS ---")
            plot_latency(df, rps, data_directory + f'/rps{rps}')
        print("\nAll plots have been generated and saved as PNG files.")
    else:
        print("No data was found. Please check the directory path and log file format.")