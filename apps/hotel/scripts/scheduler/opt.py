

# parse the time for the frontend, queue and io latency 

def split_log_file(input_file, output_file_1, output_file_2):
    """
    Splits a log file into two separate files based on a delimiter.
    
    The first output file contains the first 7 columns of the original file.
    The second output file contains the data from the 8th column onwards.

    Args:
        input_file (str): The path to the input log file.
        output_file_1 (str): The path for the first output file (7 columns).
        output_file_2 (str): The path for the second output file (remaining data).
    """
    
    # If file 1 and 2 already exist, skip processing
    import os
    if os.path.exists(output_file_1) and os.path.exists(output_file_2):
        print(f"Output files {output_file_1} and {output_file_2} already exist. Skipping processing.")
        return
    
    with open(input_file, 'r') as infile, \
         open(output_file_1, 'w') as outfile1, \
         open(output_file_2, 'w') as outfile2:
             
        first_line = infile.readline()
        parts_header = first_line.split(',', 7)
        outfile1.write(','.join(parts_header[:7]) + '\n')

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
            outfile2.write(f'{start_at},{remaining_data}')

def process_traces(trace_file):
    tasks = []
    first_arrival_time = None
    
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
            
            if first_arrival_time is None:
                first_arrival_time = arrival_time
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
    arrival_time: int
    seq: int
    task_idx: int = field(compare=False)

def _total_compute(task: Dict[str, Any]) -> int:
    return sum(seg["duration_us"] for seg in task["traces"] if seg["type"] == "Compute")

def simulate(tasks_sorted: List[Dict[str, Any]], slo: int) -> Tuple[Dict[Any, int], list]:
    if not tasks_sorted:
        return {}, []

    n = len(tasks_sorted)

    ptr = [0] * n # Pointer to the current segment in each task
    done = [False] * n # Completion status of each task
    completions: Dict[Any, int] = {} # Task ID to completion time
    ready_heap: List[ReadyItem] = [] # Min-heap of ReadyItem
    blocked_heap = [] # Min-heap of (block_finish_time, task_idx)
    timeline: List[Dict[str, Any]] = []

    arrival_times = [t["arrival_time"] for t in tasks_sorted] 
    next_arrival_ptr = 0 # Pointer to the next task arrival
    now = arrival_times[0] if arrival_times else 0 # Current simulation time
    seq_counter = 0 # Sequence counter for tie-breaking in ready heap
    remaining = n

    while remaining > 0:
        # Process new arrivals that have occurred by the current time.
        while next_arrival_ptr < n and arrival_times[next_arrival_ptr] <= now:
            tdict = tasks_sorted[next_arrival_ptr]
            traces = tdict["traces"]
            if not traces:
                done[next_arrival_ptr] = True
                remaining -= 1
                completions[tdict["id"]] = now
            else:
                seg = traces[0]
                if seg["type"] == "Block":
                    heapq.heappush(blocked_heap, (now + seg["duration_us"], next_arrival_ptr))
                    ptr[next_arrival_ptr] = 1
                else: # Compute
                    heapq.heappush(ready_heap, ReadyItem(tdict["arrival_time"], seq_counter, next_arrival_ptr))
                    seq_counter += 1
            next_arrival_ptr += 1

        # Promote tasks whose blocking period has ended by `now`.
        while blocked_heap and blocked_heap[0][0] <= now:
            _, i = heapq.heappop(blocked_heap)
            tdict = tasks_sorted[i]
            p = ptr[i]
            if p >= len(tdict["traces"]):
                done[i] = True
                remaining -= 1
                completions[tdict["id"]] = now
            else: # Must be a compute segment next
                heapq.heappush(ready_heap, ReadyItem(tdict["arrival_time"], seq_counter, i))
                seq_counter += 1

        if ready_heap:
            # Execute the highest-priority ready task.
            item = heapq.heappop(ready_heap)
            i = item.task_idx
            p = ptr[i]
            tdict = tasks_sorted[i]
            traces = tdict["traces"]
            
            seg = traces[p]
            start, dur = now, seg["duration_us"]
            now += dur
            ptr[i] += 1
            
            timeline.append({"task_id": tdict["id"], "start": start, "end": now, "duration": dur})

            if ptr[i] < len(traces): # Task has more segments
                next_seg = traces[ptr[i]]
                heapq.heappush(blocked_heap, (now + next_seg["duration_us"], i))
                ptr[i] += 1
            else: # Task is complete
                done[i] = True
                remaining -= 1
                completions[tdict["id"]] = now
        else:
            # If CPU is idle, jump time to the next event.
            next_event_time = float('inf')
            if next_arrival_ptr < n:
                next_event_time = min(next_event_time, arrival_times[next_arrival_ptr])
            if blocked_heap:
                next_event_time = min(next_event_time, blocked_heap[0][0])
            
            if next_event_time == float('inf'):
                break # No more events are possible
            now = next_event_time

    return completions, timeline

def moore_algo(tasks: List[Dict[str, Any]], slo: int) -> Dict[str, Any]:
    tasks_with_compute = [
        {**t, 'total_compute': sum(s["duration_us"] for s in t["traces"] if s["type"] == "Compute")}
        for t in tasks
    ]
    
    tasks_sorted = sorted(tasks_with_compute, key=lambda t: t["arrival_time"])
    tasks_with_ids = [{"id": i, **t} for i, t in enumerate(tasks_sorted)]
    admitted: List[Dict[str, Any]] = []
    dropped: List[Dict[str, Any]] = []

    # Add progress bar for task admission
    for t in tqdm(tasks_with_ids, desc="Moore's Algorithm Admission", unit="task"):
        admitted.append(t)
        while True:
            completions, _ = simulate(admitted, slo)
            violations = [
                tt for tt in admitted 
                if completions.get(tt["id"], float('inf')) > tt["arrival_time"] + slo
            ]
            
            if not violations:
                break
            to_remove = max(admitted, key=_total_compute)
            admitted.remove(to_remove)
            dropped.append(to_remove)

    completions, timeline = simulate(admitted, slo)
    accepted_ids = [t["id"] for t in admitted]
    dropped_ids = [t["id"] for t in dropped]

    return {
        "accepted_ids": accepted_ids,
        "dropped_ids": dropped_ids,
        "completions": completions,
        "goodput_count": len(accepted_ids),
        "goodput_ratio": (len(accepted_ids) / len(tasks)) if tasks else 0.0,
        "final_timeline": timeline,
    } 

if __name__ == "__main__":
    LOG_INPUT_PATH = "data/out/queue-experiment01/0/fifo/r400_Search.csv"
    LOG_OUTPUT_PATH_1 = "data/out/queue-experiment01/0/fifo/r400_Search_orig.csv"
    LOG_OUTPUT_PATH_2 = "data/out/queue-experiment01/0/fifo/r400_Search_trace.csv"

    # Process the file, split into two.
    split_log_file(LOG_INPUT_PATH, LOG_OUTPUT_PATH_1, LOG_OUTPUT_PATH_2)
    
    # Process the trace file to extract task traces
    task_traces = process_traces(LOG_OUTPUT_PATH_2)
    
    # Apply Moore's algorithm
    sim = moore_algo(task_traces, slo=50000)
    print(f"Goodput: {sim['goodput_count']} / {len(task_traces)} = {sim['goodput_ratio']:.2%}")
    