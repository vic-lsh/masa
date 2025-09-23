CONFIG_FILE_PATH = "data/in/queue-experiment01/gen_config.json"

def parse_json_rps_config(config_path):
    import json
    with open(config_path, 'r') as f:
        config = json.load(f)
        rps_values = config.get("Rps", [])
    return rps_values

# parse the time for the frontend, queue and io latency 
def split_log_file(input_file, output_file_1, output_file_2, output_file_3):
    """
    Splits a log file into two separate files based on a delimiter.
    
    The first output file contains the first 7 columns of the original file.
    The second output file contains the data from the 8th column onwards.

    Args:
        input_file (str): The path to the input log file.
        output_file_1 (str): The path for the first output file (7 columns).
        output_file_2 (str): The path for the second output file (remaining data).
    """
    
    import os
    
    # Terminate if the script is not run from the 'apps/hotel' directory
    if not os.getcwd().endswith('apps/hotel'):
        print(f"Current working directory: {os.getcwd()}")
        raise FileNotFoundError("Please run the script from the 'apps/hotel' directory.")
    
    first_arrival_time = None
    
    with open(input_file, 'r') as infile, \
         open(output_file_1, 'w') as outfile1, \
         open(output_file_2, 'w') as outfile2, \
         open(output_file_3, 'w') as outfile3:

        first_line = infile.readline()
        parts_header = first_line.split(',', 7)
        outfile1.write(','.join(parts_header[:7]) + '\n')
        
        search_first_arrival_time = None
        reservation_first_arrival_time = float('inf')

        for line in infile:
            # Skip empty lines
            if not line.strip():
                continue

            # Split the line by the first 7 commas
            parts_file1 = line.split(',', 7)
            
            # Write the first 7 columns to the first file
            outfile1.write(','.join(parts_file1[:7]) + '\n')

            # Write arrival time and remaining data to the second file
            start_at = parts_file1[3]
            remaining_data = parts_file1[7] if len(parts_file1) > 7 else ''

            # Sepaprate the remaining data if reservation traces exist
            if 'reservation' in remaining_data:
                trace_parts = remaining_data.split('reservation,', 1)
                remaining_data = trace_parts[0].rstrip(',')
                reservation_trace = trace_parts[1]
                reservation_first_arrival_time = min(reservation_first_arrival_time, int(reservation_trace.split(',', 1)[0]))
                outfile3.write(f'{reservation_trace}')

            outfile2.write(f'{start_at},{remaining_data}\n')

        import pandas as pd
        df1 = pd.read_csv(output_file_1)

        # print the ratio of error that is None
        ratio_none = (df1["error"] == "/None").mean()
        print(f"Goodput Ratio in FIFO: {ratio_none:.2%}")
        search_first_arrival_time = df1["start_at"].min()
        
    # return the smallest start_at time
    return search_first_arrival_time, reservation_first_arrival_time

def process_traces(trace_file, first_arrival_time):
    tasks = []
    
    # Regex to find compute and block latency
    import re
    pattern = re.compile(r'(Compute|Block)\((\d+)us\)')
    
    with open(trace_file, 'r') as f:
        for line in f:
            if not line.strip():
                continue
            
            parts = line.split(',', 1)
            arrival_time = int(parts[0])
            trace_str = parts[1] 
            
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


from dataclasses import dataclass, field
from typing import List, Dict, Any, Tuple
import heapq
from tqdm import tqdm

@dataclass(order=True)
class ReadyItem:
    # FCFS among ready tasks by original arrival order (arrival, tiebreaker)
    key: Tuple[int, int]
    task_idx: int = field(compare=False)

def simulate(tasks_sorted: List[Dict[str, Any]], slo: int, return_timeline: bool=False):
    """Single-thread compute/block simulator. tasks_sorted must be sorted by arrival_time.
       Returns:
         completions: List[int] aligned with tasks_sorted indices (completion time in us)
         timeline (optional): list of executed compute slices
    """
    n = len(tasks_sorted)
    if n == 0:
        return [], [] if return_timeline else []

    # Locals for speed
    tasks = tasks_sorted
    arrival_times = [t["arrival_time"] for t in tasks]
    ptr = [0] * n       # next segment index
    done = [False] * n
    completions = [-1] * n

    ready_heap: List[ReadyItem] = [] # (arrival, tiebreaker, task_idx)
    blocked_heap: List[Tuple[int, int]] = []  # (ready_time, task_idx)
    next_arrival = 0
    now = arrival_times[0]
    seq = 0

    if return_timeline:
        timeline: List[Dict[str, int]] = []

    # Helpers
    def push_ready(i: int):
        nonlocal seq
        heapq.heappush(ready_heap, ReadyItem((arrival_times[i], seq), i))
        seq += 1

    while True:
        # Admit arrivals up to now
        while next_arrival < n and arrival_times[next_arrival] <= now:
            i = next_arrival
            traces = tasks[i]["traces"]
            if not traces:
                done[i] = True
                completions[i] = now
            else:
                first = traces[0]
                if first["type"] == "Block":
                    # becomes ready after this block
                    heapq.heappush(blocked_heap, (now + first["duration_us"], i))
                    ptr[i] = 1
                else:
                    push_ready(i)
            next_arrival += 1

        # Promote finished blocks
        while blocked_heap and blocked_heap[0][0] <= now:
            _, i = heapq.heappop(blocked_heap)
            traces = tasks[i]["traces"]
            if ptr[i] >= len(traces):
                done[i] = True
                completions[i] = now
            else:
                # next must be Compute
                push_ready(i)

        if ready_heap:
            i = heapq.heappop(ready_heap).task_idx
            traces = tasks[i]["traces"]
            p = ptr[i]
            seg = traces[p]
            dur = seg["duration_us"]
            start = now
            now = start + dur
            ptr[i] = p + 1
            if return_timeline:
                timeline.append({"task_idx": i, "start": start, "end": now, "duration": dur})
            # Next is either Block or done
            if ptr[i] < len(traces):
                next_seg = traces[ptr[i]]
                # Expect Block per normalization
                ready_time = now + next_seg["duration_us"]
                heapq.heappush(blocked_heap, (ready_time, i))
                ptr[i] += 1
            else:
                done[i] = True
                completions[i] = now
        else:
            # Jump to next event
            if next_arrival >= n and not blocked_heap:
                break
            next_time = arrival_times[next_arrival] if next_arrival < n else float("inf")
            if blocked_heap:
                if blocked_heap[0][0] < next_time:
                    next_time = blocked_heap[0][0]
            now = next_time

    return (completions, timeline) if return_timeline else (completions, None)

def moore_algo(tasks: List[Dict[str, Any]], slo: int = 50000) -> Dict[str, Any]:
    """Return only counts; much faster by avoiding timeline & extra work. Always shows progress."""
    # Precompute total compute and sort
    for t in tasks:
        t["total_compute"] = sum(s["duration_us"] for s in t["traces"] if s["type"] == "Compute")
        t["total_block"] = sum(s["duration_us"] for s in t["traces"] if s["type"] == "Block")
        t["total_time"] = sum(s["duration_us"] for s in t["traces"])
    tasks_sorted = sorted(tasks, key=lambda t: t["arrival_time"])

    admitted = []  # list of task dicts, has invariant of being feasible
    
    from tqdm import tqdm
    iterator = tqdm(list(enumerate(tasks_sorted)), desc="Admission", unit="task")

    for _, t in iterator:
        if t["total_time"] > slo:
            continue
        admitted.append(t)
        while True:
            completions, _ = simulate(admitted, slo, return_timeline=False)
            # Check violations
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
    """
    Reads a log file, sorts its lines based on the initial timestamp,
    and writes the result to a new file.
    """
    try:
        with open(input_path, 'r') as f:
            lines = f.readlines()
        lines.sort(key=lambda line: int(line.split(',', 1)[0]))
        with open(output_path, 'w') as f:
            f.writelines(lines)
        print(f"Successfully sorted '{input_path}' into '{output_path}'.")
    except FileNotFoundError:
        print(f"Error: Input file not found at '{input_path}'")
    except Exception as e:
        print(f"An error occurred: {e}")

def frontend_opt():
    # Read json config file for rps values
    rps_values = parse_json_rps_config(CONFIG_FILE_PATH)

    result_arr = []
    for rps in rps_values:
        log_input_path = f"data/out/queue-experiment01/0/fifo/r{rps}_Search.csv"
        
        log_output_path_1 = f"data/out/queue-experiment01/0/fifo/r{rps}_Search_orig.csv"
        log_output_path_2 = f"data/out/queue-experiment01/0/fifo/r{rps}_Search_trace.csv"
        
        log_output_path_2_sorted = f"data/out/queue-experiment01/0/fifo/r{rps}_Search_trace_sorted.csv"
        sort_log_file_by_timestamp(log_output_path_2, log_output_path_2_sorted)
        
        log_output_path_3 = f"data/out/queue-experiment01/0/fifo/r{rps}_reservation_trace.csv"

        first_arrival_time, _ = split_log_file(log_input_path, log_output_path_1, log_output_path_2_sorted, log_output_path_3)
        print(f"First arrival time recorded: {first_arrival_time}")
        
        # Process the trace file to extract task traces
        task_traces = process_traces(log_output_path_2_sorted, first_arrival_time)

        # Apply Moore's algorithm
        sim = moore_algo(task_traces, slo=50000)
        # Print dropped and goodput counts
        print(f"Offline Scheduler Dropped Count: {sim['dropped_count']}")
        print(f"Offline Scheduler Goodput Ratio: {sim['goodput_count']} / {len(task_traces)} = {sim['goodput_count'] / len(task_traces):.2%}")
        result_arr.append({
            "rps": rps,
            "dropped_count": sim['dropped_count'],
            "goodput_count": sim['goodput_count'],
            "total_count": len(task_traces),
            "goodput_ratio": sim['goodput_count'] / len(task_traces) if len(task_traces) > 0 else 0
        })
    print(result_arr)


def reservation_opt():
    rps_values = parse_json_rps_config(CONFIG_FILE_PATH)

    result_arr = []
    for rps in rps_values:
        log_input_path = f"data/out/queue-experiment01/0/fifo/r{rps}_Search.csv"
        log_output_path_1 = f"data/out/queue-experiment01/0/fifo/r{rps}_Search_orig.csv"
        log_output_path_2 = f"data/out/queue-experiment01/0/fifo/r{rps}_Search_trace.csv"
        log_output_path_3 = f"data/out/queue-experiment01/0/fifo/r{rps}_reservation_trace.csv"

        # Process the file, split into two.
        _, first_arrival_time = split_log_file(log_input_path, log_output_path_1, log_output_path_2, log_output_path_3)
        print(f"First arrival time recorded: {first_arrival_time}")
        
        # Process the trace file to extract task traces
        task_traces = process_traces(log_output_path_3, first_arrival_time)

        # Apply Moore's algorithm
        sim = moore_algo(task_traces, slo=50000)
        # Print dropped and goodput counts
        print(f"Offline Scheduler Dropped Count: {sim['dropped_count']}")
        print(f"Offline Scheduler Goodput Ratio: {sim['goodput_count']} / {len(task_traces)} = {sim['goodput_count'] / len(task_traces):.2%}")
        result_arr.append({
            "rps": rps,
            "dropped_count": sim['dropped_count'],
            "goodput_count": sim['goodput_count'],
            "total_count": len(task_traces),
            "goodput_ratio": sim['goodput_count'] / len(task_traces) if len(task_traces) > 0 else 0
        })
    print(result_arr)

if __name__ == "__main__":
    frontend_opt()
    # reservation_opt()