#!/usr/bin/env python3
"""
cycle_check.py — detect a cycle in a directed call graph from a CSV.

The CSV is expected to have a header with columns "caller" and "callee".
(You can rename these via CLI flags; common typo "calee" is auto-handled.)

Usage:
  python cycle_check.py path/to/edges.csv
  python cycle_check.py edges.csv --caller-col caller --callee-col callee
  python cycle_check.py edges.csv --delimiter ';'
  python cycle_check.py edges.csv --ignore-self-loops

Exit codes:
  0 => no cycle
  1 => cycle exists or input error (details printed to stderr)
"""

import sys
import csv
import argparse
from collections import defaultdict

def read_edges(csv_path, caller_col, callee_col, delimiter, ignore_self_loops):
    adj = defaultdict(list)
    nodes = set()
    m = 0

    with open(csv_path, newline="") as f:
        # Sniff dialect but allow explicit delimiter to override
        if delimiter is None:
            try:
                sample = f.read(2048)
                f.seek(0)
                sniffer = csv.Sniffer()
                dialect = sniffer.sniff(sample)
            except csv.Error:
                dialect = csv.excel
            reader = csv.DictReader(f, dialect=dialect)
        else:
            reader = csv.DictReader(f, delimiter=delimiter)

        # Auto-handle common typo
        fieldnames_lower = {name.lower(): name for name in reader.fieldnames or []}
        if caller_col.lower() not in fieldnames_lower:
            sys.stderr.write(f"ERROR: caller column '{caller_col}' not found in CSV header {reader.fieldnames}\n")
            return None, None, None
        if callee_col.lower() not in fieldnames_lower:
            # try 'calee'
            if "calee" in fieldnames_lower and callee_col.lower() != "calee":
                sys.stderr.write("NOTE: using 'calee' column (common typo) instead of requested 'callee'.\n")
                callee_col = "calee"
            else:
                sys.stderr.write(f"ERROR: callee column '{callee_col}' not found in CSV header {reader.fieldnames}\n")
                return None, None, None

        caller_key = fieldnames_lower[caller_col.lower()]
        callee_key = fieldnames_lower[callee_col.lower()]

        for i, row in enumerate(reader, start=2):  # start=2 (line after header) for friendlier messages
            try:
                u = (row.get(caller_key) or "").strip()
                v = (row.get(callee_key) or "").strip()
            except Exception as e:
                sys.stderr.write(f"ERROR: failed to parse row {i}: {e}\n")
                return None, None, None

            if not u or not v:
                # skip empty endpoints
                continue
            if ignore_self_loops and u == v:
                continue

            nodes.add(u); nodes.add(v)
            adj[u].append(v)
            m += 1

    # Ensure all nodes appear in adjacency (even sinks)
    for n in list(nodes):
        adj.setdefault(n, [])

    return adj, len(nodes), m

def find_cycle(adj):
    """
    DFS with 3-color marking.
    Returns:
      (True, cycle_list) if a cycle is found (cycle_list is node sequence, first==last),
      (False, None) otherwise.
    """
    WHITE, GRAY, BLACK = 0, 1, 2
    color = {u: WHITE for u in adj}
    parent = {}

    def dfs(start):
        stack = [start]
        path_stack = []        # explicit stack for current path simulation
        iter_stack = []

        while stack:
            u = stack.pop()
            if color[u] == WHITE:
                color[u] = GRAY
                path_stack.append(u)
                iter_stack.append(iter(iter(adj[u])))
                # Re-push u to handle post-processing when children done
                stack.append(u)
                # push first child (if any)
                try:
                    v = next(iter_stack[-1])
                    # Process one neighbor at a time
                    stack.append(v)
                    parent[v] = u
                except StopIteration:
                    # no children
                    pass
            elif color[u] == GRAY:
                # We're returning to u to continue its adjacency iteration
                try:
                    v = next(iter_stack[-1])
                    stack.append(u)  # we'll come back again
                    stack.append(v)
                    parent[v] = u
                except StopIteration:
                    # done with u's children
                    color[u] = BLACK
                    path_stack.pop()
                    iter_stack.pop()
            else:
                # BLACK: nothing to do
                continue

            # While processing neighbors, detect back edges
            if stack:
                top = stack[-1]
                if top in adj:  # only nodes, not iterators
                    # If we're about to visit a neighbor already GRAY, we found a cycle
                    if color.get(top, WHITE) == GRAY:
                        # Reconstruct cycle from top back to top via parents
                        cycle = [top]
                        cur = parent.get(top)
                        while cur is not None and cur != top:
                            cycle.append(cur)
                            cur = parent.get(cur)
                            # safety against malformed parent chains
                            if len(cycle) > len(adj) + 1:
                                break
                        if cur == top:
                            cycle.append(top)
                            cycle.reverse()
                            return True, cycle
        return False, None

    for u in adj:
        if color[u] == WHITE:
            ok, cyc = dfs(u)
            if ok:
                return True, cyc
    return False, None

def main():
    p = argparse.ArgumentParser(description="Detect a cycle in a directed graph from a CSV with caller/callee columns.")
    p.add_argument("csv", help="Path to CSV file with edges.")
    p.add_argument("--caller-col", default="caller", help="Column name for caller (default: caller)")
    p.add_argument("--callee-col", default="callee", help="Column name for callee (default: callee)")
    p.add_argument("--delimiter", default=None, help="CSV delimiter (default: auto-detect)")
    p.add_argument("--ignore-self-loops", action="store_true", help="Ignore u->u edges as cycles")
    args = p.parse_args()

    adj, n_nodes, n_edges = read_edges(
        args.csv, args.caller_col, args.callee_col, args.delimiter, args.ignore_self_loops
    )
    if adj is None:
        sys.exit(1)

    has_cycle, cycle = find_cycle(adj)

    print(f"Nodes: {n_nodes}, Edges: {n_edges}")
    if has_cycle:
        # Pretty print a compact cycle (avoid repeating the last node twice in the arrow sequence)
        seq = " -> ".join(cycle[:-1]) + f" -> {cycle[-1]} (back to start)"
        print("Cycle: YES")
        print("One cycle found:")
        print(seq)
        sys.exit(1)
    else:
        print("Cycle: NO")
        sys.exit(0)

if __name__ == "__main__":
    main()