import json
import re


def parse_internal_spans(fields, start_index, span_pattern):
    """Parses consecutive spans for a child service."""
    spans = []
    i = start_index
    while i < len(fields):
        match = span_pattern.match(fields[i])
        if not match:
            # Stop when we encounter a non-span field
            break
        
        span_type, latency_value = match.groups()
        if span_type in ["Compute", "Block"]:
            spans.append({
                "type": span_type,
                "latency_us": int(latency_value)
            })
        i += 1
    return spans, i

def parse_trace_log_to_json(log_data):
    """
    Parses the trace log, intelligently identifying child calls and nesting them
    within the parent's span timeline.
    """
    all_traces = []
    span_pattern = re.compile(r"(\w+)\((\d+)us\)")

    for line in log_data.strip().split('\n'):
        fields = line.strip().split(',')
        
        root_trace = {
            "service_name": fields[0],
            "spans": [], # The main timeline of events
        }

        i = 7  # Start reading from the first potential span
        
        while i < len(fields):
            field = fields[i]
            match = span_pattern.match(field)

            if not match:
                i += 1
                continue

            span_type, latency_value = match.groups()

            # --- Smart Lookahead Logic ---
            # Check if the next field is a service name (i.e., not another span)
            is_child_call = False
            if (i + 1) < len(fields):
                # If the next field does NOT match the span pattern, it's a service name
                if not span_pattern.match(fields[i+1]):
                    is_child_call = True

            # If this is a Block span that precedes a child service, create a ChildCall object
            if span_type == "Block" and is_child_call:
                child_name = fields[i+1]
                child_timestamp = fields[i+2]
                
                # The child's own internal spans start after its name and timestamp
                child_spans, next_i = parse_internal_spans(fields, i + 3, span_pattern)

                root_trace["spans"].append({
                    "type": "ChildCall",
                    "service_name": child_name,
                    "latency_us": int(latency_value),
                    "start_timestamp": child_timestamp,
                    "spans": child_spans
                })
                i = next_i # Jump the index ahead past all the child's spans
            else:
                # This is a regular local span for the parent service
                if span_type in ["Compute", "Block"]:
                    root_trace["spans"].append({
                        "type": span_type,
                        "latency_us": int(latency_value)
                    })
                i += 1

        all_traces.append(root_trace)

    return json.dumps(all_traces, indent=2)


# --- Run the conversion and print the new, improved JSON ---
if __name__ == "__main__":
    DATA_INTPUT = "data/out/queue-experiment01/0/fifo/r1050_Search.csv"
    JSON_OUTPUT = "data/out/queue-experiment01/0/fifo/r1050_Search_parsed.json"
    
    json_output = parse_trace_log_to_json(open(DATA_INTPUT).read())
    with open(JSON_OUTPUT, 'w') as f:
        f.write(json_output)
    print(f"Converted trace log saved to {JSON_OUTPUT}")