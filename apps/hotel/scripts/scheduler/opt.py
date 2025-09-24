import json
import os
import re
import pandas as pd
from contextlib import ExitStack
from dataclasses import dataclass, field
from typing import List, Dict, Any, Tuple
import heapq
from tqdm import tqdm

# Ensure the config file path is correct for your environment
CONFIG_FILE_PATH = "data/in/queue-experiment01/gen_config.json"

def parse_json_rps_config(config_path):
    """Reads RPS values from a JSON configuration file."""
    with open(config_path, 'r') as f:
        config = json.load(f)
        rps_values = config.get("Rps", [])
    return rps_values

def split_log_file(input_file, output_file_columns, keyword_output_paths):
    """
    Splits a combined log file into a primary columns file and separate trace files for each service.

    This function now correctly handles the 'frontend' trace data, which appears before any
    other service keywords in the log line.

    Args:
        input_file (str): Path to the source log file with combined traces.
        output_file_columns (str): Path to write the first seven columns of the log.
        keyword_output_paths (Dict[str, str]): A mapping of service keywords (e.g., "frontend", "search")
                                             to their respective output file paths.

    Returns:
        Dict[str, int | None]: A dictionary of the first arrival times found for each service.
    """
    if not keyword_output_paths:
        raise ValueError("keyword_output_paths must contain at least one entry")

    # The regex pattern should match downstream services to find where the frontend trace ends.
    downstream_keywords = [k for k in keyword_output_paths if k != "frontend"]
    pattern = re.compile(rf"({'|'.join(re.escape(k) for k in downstream_keywords)}),")
    
    keyword_first_arrival = {k: float('inf') for k in keyword_output_paths}

    with ExitStack() as stack:
        infile = stack.enter_context(open(input_file, 'r'))
        outfile_cols = stack.enter_context(open(output_file_columns, 'w'))
        keyword_files = {
            key: stack.enter_context(open(path, 'w'))
            for key, path in keyword_output_paths.items()
        }

        # Process the header line first
        first_line = infile.readline()
        if not first_line:
            return {k: None for k in keyword_output_paths}
        
        header_parts = first_line.rstrip('\n').split(',', 7)
        outfile_cols.write(','.join(header_parts[:7]) + '\n')

        # Process each data line in the log file
        for line in infile:
            if not line.strip():
                continue

            parts = line.rstrip('\n').split(',', 7)
            outfile_cols.write(','.join(parts[:7]) + '\n')
            
            if len(parts) <= 7:
                continue

            remaining_data = parts[7]
            matches = list(pattern.finditer(remaining_data))
            
            # --- Handle Frontend Trace ---
            if "frontend" in keyword_files:
                first_match_start = matches[0].start() if matches else len(remaining_data)
                frontend_trace = remaining_data[:first_match_start].strip(', \n')
                
                if frontend_trace:
                    frontend_arrival_time = parts[3]  # Use 'start_at' as arrival time
                    keyword_files["frontend"].write(f"{frontend_arrival_time},{frontend_trace}\n")
                    
                    try:
                        arrival_val = int(frontend_arrival_time)
                        keyword_first_arrival["frontend"] = min(keyword_first_arrival["frontend"], arrival_val)
                    except ValueError:
                        pass
            
            # --- Handle Downstream Service Traces (search, reservation, etc.) ---
            for idx, match in enumerate(matches):
                keyword = match.group(1)
                start_idx = match.end()
                end_idx = matches[idx + 1].start() if idx + 1 < len(matches) else len(remaining_data)
                segment = remaining_data[start_idx:end_idx].strip(', \n')
                
                if segment:
                    keyword_files[keyword].write(f"{segment}\n")
                    
                    first_token = segment.split(',', 1)[0].strip()
                    try:
                        arrival_time = int(first_token)
                        keyword_first_arrival[keyword] = min(keyword_first_arrival[keyword], arrival_time)
                    except ValueError:
                        continue
    
    # Calculate goodput ratio from the main columns file
    try:
        df_columns = pd.read_csv(output_file_columns)
        if not df_columns.empty and "error" in df_columns.columns:
            ratio_none = (df_columns["error"] == "/None").mean()
            print(f"Goodput Ratio in FIFO: {ratio_none:.2%}")
    except (FileNotFoundError, pd.errors.EmptyDataError):
        print(f"Warning: Could not read columns file at {output_file_columns}")

    # Prepare final results for first arrival times
    result = {
        keyword: None if first_time == float('inf') else first_time
        for keyword, first_time in keyword_first_arrival.items()
    }
    return result

def process_traces(trace_file, first_arrival_time):
    """Parses a service-specific trace file to extract compute and block tasks."""
    tasks = []
    pattern = re.compile(r'(Compute|Block)\((\d+)us\)')
    
    with open(trace_file, 'r') as f:
        for line in f:
            if not line.strip():
                continue
            
            parts = line.split(',', 1)
            arrival_time = int(parts[0])
            trace_str = parts[1]
            
            # Normalize arrival time
            arrival_time -= first_arrival_time
            
            matches = pattern.findall(trace_str)
            
            task_traces = []
            current_compute_time = 0
            
            for type, duration in matches:
                duration_us = int(duration)
                if type == 'Compute':
                    current_compute_time += duration_us
                elif type == 'Block':
                    if current_compute_time > 0:
                        task_traces.append({'type': 'Compute', 'duration_us': current_compute_time})
                        current_compute_time = 0
                    task_traces.append({'type': 'Block', 'duration_us': duration_us})

            if current_compute_time > 0:
                task_traces.append({'type': 'Compute', 'duration_us': current_compute_time})
            
            tasks.append({'arrival_time': arrival_time, 'traces': task_traces})
    return tasks

@dataclass(order=True)
class ReadyItem:
    """Helper class for the simulation's priority queue."""
    key: Tuple[int, int]
    task_idx: int = field(compare=False)

def simulate(tasks_sorted: List[Dict[str, Any]], slo: int, return_timeline: bool = False):
    """Single-thread compute/block simulator. tasks_sorted must be sorted by arrival_time."""
    n = len(tasks_sorted)
    if n == 0:
        return [], [] if return_timeline else []

    tasks = tasks_sorted
    arrival_times = [t["arrival_time"] for t in tasks]
    ptr = [0] * n
    done = [False] * n
    completions = [-1] * n

    ready_heap: List[ReadyItem] = []
    blocked_heap: List[Tuple[int, int]] = []
    next_arrival = 0
    now = arrival_times[0]
    seq = 0
    timeline: List[Dict[str, int]] = [] if return_timeline else None

    def push_ready(i: int):
        nonlocal seq
        heapq.heappush(ready_heap, ReadyItem((arrival_times[i], seq), i))
        seq += 1

    while True:
        while next_arrival < n and arrival_times[next_arrival] <= now:
            i = next_arrival
            traces = tasks[i]["traces"]
            if not traces:
                done[i], completions[i] = True, now
            else:
                first = traces[0]
                if first["type"] == "Block":
                    heapq.heappush(blocked_heap, (now + first["duration_us"], i))
                    ptr[i] = 1
                else:
                    push_ready(i)
            next_arrival += 1

        while blocked_heap and blocked_heap[0][0] <= now:
            _, i = heapq.heappop(blocked_heap)
            traces = tasks[i]["traces"]
            if ptr[i] >= len(traces):
                done[i], completions[i] = True, now
            else:
                push_ready(i)

        if ready_heap:
            i = heapq.heappop(ready_heap).task_idx
            traces, p = tasks[i]["traces"], ptr[i]
            seg, dur = traces[p], traces[p]["duration_us"]
            start, now = now, now + dur
            ptr[i] = p + 1
            if return_timeline:
                timeline.append({"task_idx": i, "start": start, "end": now, "duration": dur})

            if ptr[i] < len(traces):
                next_seg = traces[ptr[i]]
                ready_time = now + next_seg["duration_us"]
                heapq.heappush(blocked_heap, (ready_time, i))
                ptr[i] += 1
            else:
                done[i], completions[i] = True, now
        else:
            if next_arrival >= n and not blocked_heap:
                break
            next_time = arrival_times[next_arrival] if next_arrival < n else float("inf")
            if blocked_heap and blocked_heap[0][0] < next_time:
                next_time = blocked_heap[0][0]
            now = next_time

    return (completions, timeline) if return_timeline else (completions, None)

def moore_algo(tasks: List[Dict[str, Any]], slo: int = 50000) -> Dict[str, Any]:
    """Applies Moore's algorithm for admission control to maximize goodput."""
    for t in tasks:
        t["total_time"] = sum(s["duration_us"] for s in t["traces"])
    tasks_sorted = sorted(tasks, key=lambda t: t["arrival_time"])

    admitted = []
    iterator = tqdm(list(enumerate(tasks_sorted)), desc="Admission", unit="task", leave=False)

    for _, t in iterator:
        if t["total_time"] > slo:
            continue
        admitted.append(t)
        while True:
            completions, _ = simulate(admitted, slo, return_timeline=False)
            ok = True
            for i, tt in enumerate(admitted):
                if completions[i] == -1 or completions[i] > tt["arrival_time"] + slo:
                    ok = False
                    break
            if ok:
                break
            idx = max(range(len(admitted)), key=lambda k: admitted[k]["total_time"])
            admitted.pop(idx)

    goodput = len(admitted)
    dropped = len(tasks_sorted) - goodput
    return {"goodput_count": goodput, "dropped_count": dropped}

def sort_log_file_by_timestamp(input_path, output_path):
    """Reads a log file, sorts it by the initial timestamp, and writes to a new file."""
    try:
        with open(input_path, 'r') as f:
            lines = [line for line in f if line.strip()]
        lines.sort(key=lambda line: int(line.split(',', 1)[0]))
        with open(output_path, 'w') as f:
            f.writelines(lines)
        print(f"Successfully sorted '{input_path}' into '{output_path}'.")
    except FileNotFoundError:
        print(f"Error: Input file not found at '{input_path}'")
    except Exception as e:
        print(f"An error occurred during sorting: {e}")

if __name__ == "__main__":
    rps_values = parse_json_rps_config(CONFIG_FILE_PATH)
    
    services = ["frontend", "search", "reservation", "profile"]
    all_results = {service: [] for service in services}

    for rps in rps_values:
        print(f"\n==================== PROCESSING RPS: {rps} ====================")
        
        # Define paths for input and output files for the current RPS
        log_input_path = f"data/out/queue-experiment01/0/fifo/r{rps}_Search.csv"
        columns_output_path = f"data/out/queue-experiment01/0/fifo/r{rps}_columns.csv"
        
        # Define output paths for individual service traces, matching your original names
        trace_output_paths = {
            "frontend": f"data/out/queue-experiment01/0/fifo/r{rps}_frontend_trace.csv",
            "search": f"data/out/queue-experiment01/0/fifo/r{rps}_Search_trace.csv", # Capital 'S'
            "reservation": f"data/out/queue-experiment01/0/fifo/r{rps}_reservation_trace.csv",
            "profile": f"data/out/queue-experiment01/0/fifo/r{rps}_profile_trace.csv",
        }
        
        # 1. Split the combined log file ONCE for this RPS value.
        print(f"Splitting combined log file: {log_input_path}")
        arrival_times = split_log_file(
            log_input_path,
            columns_output_path,
            trace_output_paths,
        )
        print("First arrival times recorded:", arrival_times)
        
        # 2. Run optimization analysis for each service using the generated trace files.
        for service_name in services:
            first_arrival = arrival_times.get(service_name)
            
            if first_arrival is not None:
                print(f"\n--- Optimizing for Service: {service_name.capitalize()} ---")
                
                trace_path = trace_output_paths[service_name]
                sorted_trace_path = trace_path.replace(".csv", "_sorted.csv")
                
                sort_log_file_by_timestamp(trace_path, sorted_trace_path)
                task_traces = process_traces(sorted_trace_path, first_arrival)
                
                if not task_traces:
                    print(f"No tasks to process for '{service_name}'.")
                    continue
                
                sim_results = moore_algo(task_traces, slo=50000)
                
                total_tasks = len(task_traces)
                goodput_count = sim_results['goodput_count']
                goodput_ratio = goodput_count / total_tasks if total_tasks > 0 else 0

                print(f"Offline Scheduler Dropped Count: {sim_results['dropped_count']}")
                print(f"Offline Scheduler Goodput: {goodput_count} / {total_tasks} = {goodput_ratio:.2%}")
                
                result = {
                    "rps": rps,
                    "service": service_name,
                    "dropped_count": sim_results['dropped_count'],
                    "goodput_count": goodput_count,
                    "total_count": total_tasks,
                    "goodput_ratio": goodput_ratio
                }
                all_results[service_name].append(result)
            else:
                print(f"\n--- No traces found for Service: {service_name.capitalize()}. Skipping. ---")

    print("\n\n==================== FINAL SUMMARY ====================")
    for service, results in all_results.items():
        print(f"\n=== Results for {service.capitalize()} ===")
        print(json.dumps(results, indent=2))