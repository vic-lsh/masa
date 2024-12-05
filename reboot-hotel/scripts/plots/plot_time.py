import json
import os
from typing import *

import pandas as pd
from plot_core import parse_args, plot_time_bar  # type: ignore

MS = 1e3

args = parse_args()
cfg = json.load(open(args.gen_config))

r = 0
rps_to_results: List[Dict[str, Any]] = []


def plot_total() -> None:
    for rps in cfg["Rps"]:
        file = f"{args.path}/r{rps}_{r}.csv"
        df = pd.read_csv(file)

        result = {}
        result["rps"] = rps

        df_filtered = df[df["error"] == "/None"]
        if df_filtered.empty:
            print(f"Got empty data for good_total, rps: {rps}")
            result["good_total"] = 0
        else:
            result["good_total"] = sum(df_filtered["latency"])

        df_filtered = df[df["error"].str.contains("EarlyReturn")]
        if df_filtered.empty:
            print(f"Got empty data for err_svc_er_total, rps: {rps}")
            result["err_svc_er_total"] = 0
        else:
            result["err_svc_er_total"] = sum(df_filtered["latency"])

        df_filtered = df[df["error"] == "/ClientMiss"]
        if df_filtered.empty:
            print(f"Got empty data for err_client_miss_total, rps: {rps}")
            result["err_cl_miss_total"] = 0
        else:
            result["err_cl_miss_total"] = sum(df_filtered["latency"])

        rps_to_results.append(result)

    plot_time_bar(
        rps_to_results,
        f"{args.path}/time/fig_time_total_rps_bar_{r}.png",
        f"{args.mode} total",
    )


# plot_total()


def get_time_breakdown(data):
    df = pd.DataFrame(data)

    # Calculate basic statistics
    basic_stats = df.agg(["mean", "std"]).astype(int)
    basic_stats.index = ["mean", "std"]

    # Calculate percentiles
    percentiles = df.describe(percentiles=[0.50, 0.90, 0.95, 0.99])
    percentile_stats = percentiles.loc[["50%", "90%", "95%", "99%"]].astype(int)
    percentile_stats.index = ["p50", "p90", "p95", "p99"]

    # Combine all statistics
    combined_stats = pd.concat([basic_stats, percentile_stats])

    # Calculate percentages and format output
    pct_data = {}
    for col in ["processing_lat", "queueing_lat"]:
        pct_data[col] = []
        for idx in combined_stats.index:
            absolute = combined_stats.loc[idx, col]
            total = combined_stats.loc[idx, "total_lat"]
            percentage = (absolute / total * 100).round(1)
            pct_data[col].append(f"{absolute} ({percentage}%)")

    # Create final result DataFrame
    result = pd.DataFrame(
        {
            "processing_lat": pct_data["processing_lat"],
            "queueing_lat": pct_data["queueing_lat"],
            "total_lat": combined_stats["total_lat"],
        },
        index=combined_stats.index,
    )

    # Reorder index to put mean and std first, then percentiles
    desired_order = ["mean", "std", "p50", "p90", "p95", "p99"]
    result = result.reindex(desired_order)

    return result


def get_service_breakdown(results_dict, stat_func):
    # Calculate statistics for each service
    service_stats = {}
    for service, data in results_dict.items():
        df = pd.DataFrame(data)
        stats = stat_func(df).astype(int)
        service_stats[service] = stats

    # Prepare data for final breakdown table
    breakdown_data = []
    frontend_values = service_stats["frontend"]

    for service, values in service_stats.items():
        queue_pct = (
            values["queueing_lat"] / frontend_values["queueing_lat"] * 100
        ).round(1)
        total_pct = (values["total_lat"] / frontend_values["total_lat"] * 100).round(1)

        # Format output strings:
        # - Processing latency: absolute value only
        # - Queueing latency: absolute value and percentage of frontend queueing
        # - Total latency: absolute value and percentage of frontend total
        proc_str = f"{values['processing_lat']}"
        queue_str = f"{values['queueing_lat']} ({queue_pct}%)"
        total_str = f"{values['total_lat']} ({total_pct}%)"

        # Create row entry for current service
        row = {
            "service": service,
            "processing_lat": proc_str,
            "queueing_lat": queue_str,
            "total_lat": total_str,
        }
        breakdown_data.append(row)

    # Convert to DataFrame and set service name as index
    return pd.DataFrame(breakdown_data).set_index("service")


def write_csv_with_header(df, filename, header_text):
    os.makedirs(os.path.dirname(filename), exist_ok=True)
    with open(filename, "w") as f:
        f.write(f"# {header_text}\n\n")
        f.write(df.to_string())
        f.write("\n")


def plot_breakdown() -> None:
    assert len(cfg["Rps"]) == 1
    output_path = f"{args.path}/time"
    services = ["frontend", "search", "profile", "reservation", "geo", "rate", "user"]
    service_to_results = {}

    for svc in services:
        file = f"{args.path}/tmp_hotel_{svc}.log"

        with open(file) as f:
            lines = f.readlines()
            lines = list(
                filter(
                    lambda x: "simple.rs" in x and "finalize" in x and "_lat" in x,
                    lines,
                )
            )
            results: List = []
            for line in lines:
                parts = line.split(",")
                result = {}
                for part in parts:
                    if "_lat" in part:
                        key, value = part.split(":")
                        key = key.strip()
                        value = value.strip()
                        result[key] = int(value)
                results.append(result)

        service_to_results[svc] = results

        time_breakdown = get_time_breakdown(results)
        file = f"{output_path}/{svc}/table_time_breakdown.csv"
        write_csv_with_header(
            time_breakdown,
            file,
            f"Time breakdown of {svc}",
        )

    mean_breakdown = get_service_breakdown(service_to_results, lambda df: df.mean())
    write_csv_with_header(
        mean_breakdown,
        f"{output_path}/all/table_service_breakdown_mean.csv",
        "Service breakdown relative to frontend (mean)",
    )

    percentiles = [50, 90, 95, 99]
    for p in percentiles:
        pct_breakdown = get_service_breakdown(
            service_to_results, lambda df: df.quantile(p / 100)
        )
        write_csv_with_header(
            pct_breakdown,
            f"{output_path}/all/table_service_breakdown_p{p}.csv",
            f"Service breakdown relative to frontend (p{p})",
        )


plot_breakdown()
