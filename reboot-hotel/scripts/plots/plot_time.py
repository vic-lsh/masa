import json
import os
from typing import *

import pandas as pd
from plot_core import parse_args, plot_time_bar  # type: ignore

MS = 1e3

args = parse_args()
cfg = json.load(open(args.gen_config))
r = 0


def write_csv(df, header, filename):
    os.makedirs(os.path.dirname(filename), exist_ok=True)
    with open(filename, "w") as f:
        f.write(f"# {header}\n\n")
        f.write(df.to_string())
        f.write("\n")


def write_multiple_csvs(results: List[Tuple[pd.DataFrame, str]], filename: str):
    os.makedirs(os.path.dirname(filename), exist_ok=True)
    with open(filename, "w") as f:
        for df, header in results:
            f.write(f"# {header}\n\n")
            f.write(df.to_string())
            f.write("\n\n")


def get_request_breakdown(result: Dict) -> Any:
    # Calculate total sum
    total = (
        result["good_total"] + result["err_svc_er_total"] + result["err_cl_miss_total"]
    )

    # Calculate percentages if total is not zero
    if total > 0:
        result["good_total_pct"] = (result["good_total"] / total) * 100
        result["err_svc_er_total_pct"] = (result["err_svc_er_total"] / total) * 100
        result["err_cl_miss_total_pct"] = (result["err_cl_miss_total"] / total) * 100
    else:
        result["good_total_pct"] = 0
        result["err_svc_er_total_pct"] = 0
        result["err_cl_miss_total_pct"] = 0

    # Create new format data
    data = {
        "type": ["good_total", "err_svc_er_total", "err_cl_miss_total"],
        "sum": [
            f'{result["good_total"]} ({result["good_total_pct"]:.1f}%)',
            f'{result["err_svc_er_total"]} ({result["err_svc_er_total_pct"]:.1f}%)',
            f'{result["err_cl_miss_total"]} ({result["err_cl_miss_total_pct"]:.1f}%)',
        ],
    }
    return pd.DataFrame(data)


def analyze_client() -> None:
    assert len(cfg["Rps"]) == 1
    rps = cfg["Rps"][0]
    file = f"{args.path}/r{rps}_{r}.csv"
    df = pd.read_csv(file)

    result = {}

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

    df = get_request_breakdown(result)
    output_file = f"{args.path}/time/all/table_request_breakdown.csv"
    header_text = f"Request type breakdown, latency=total"
    write_csv(df, header_text, output_file)


analyze_client()


def get_time_breakdown(data):
    if not data:
        return pd.DataFrame()

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


def analyze_service_logs() -> None:
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
            type_to_results: Dict[str, List] = {
                "all": [],
                "non_err_svc_er": [],
                "err_svc_er": [],
            }
            for line in lines:
                parts = line.split(",")
                result = {}
                check_early_return = None
                for part in parts:
                    if "_lat" in part:
                        type, value = part.split(":")
                        type = type.strip()
                        value = value.strip()
                        result[type] = int(value)
                    elif "check_early_return" in part:
                        if "true" in part:
                            check_early_return = True
                        else:
                            check_early_return = False
                type_to_results["all"].append(result)
                assert check_early_return is not None
                if check_early_return:
                    type_to_results["err_svc_er"].append(result)
                else:
                    type_to_results["non_err_svc_er"].append(result)

        service_to_results[svc] = type_to_results["all"]

        output_csvs: List[Tuple[pd.DataFrame, str]] = []
        for type, results in type_to_results.items():
            time_breakdown = get_time_breakdown(results)
            output_csvs.append(
                (time_breakdown, f"Time breakdown, svc={svc}, type={type}")
            )
        write_multiple_csvs(
            output_csvs, f"{output_path}/{svc}/table_time_breakdown.csv"
        )

    output_csvs = []
    mean_breakdown = get_service_breakdown(service_to_results, lambda df: df.mean())
    output_csvs.append(
        (mean_breakdown, "Service breakdown relative to frontend (mean)")
    )
    percentiles = [50, 90, 95, 99]
    for p in percentiles:
        pct_breakdown = get_service_breakdown(
            service_to_results, lambda df: df.quantile(p / 100)
        )
        output_csvs.append(
            (pct_breakdown, f"Service breakdown relative to frontend (p{p})")
        )
    write_multiple_csvs(output_csvs, f"{output_path}/all/table_service_breakdown.csv")


analyze_service_logs()
