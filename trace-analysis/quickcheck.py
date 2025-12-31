#!/usr/bin/env python
# coding: utf-8
from __future__ import annotations

from pathlib import Path
import pandas as pd
from concurrent.futures import ProcessPoolExecutor, as_completed
from functools import partial
import sys
import logging
import argparse
from tqdm import tqdm
import networkx as nx
import matplotlib
matplotlib.use("Agg")  # Headless/parallel-safe plotting
import matplotlib.pyplot as plt
import matplotlib.patches as mpatches
from matplotlib.collections import LineCollection
import copy
import re
import numpy as np
from collections import defaultdict

# Set up logger
logger = logging.getLogger(__name__)

class TqdmLoggingHandler(logging.Handler):
    """Logging handler that uses tqdm.write() to avoid interfering with progress bars."""
    def emit(self, record):
        try:
            msg = self.format(record)
            tqdm.write(msg, file=sys.stderr)
        except Exception:
            self.handleError(record)

# ----------------------------
# Parallel CSV loading (processes)
# ----------------------------

def _sample_traces_from_df(
    df: pd.DataFrame,
    fraction: float,
    trace_col: str = "traceid",
    random_state: int | None = None,
) -> pd.DataFrame:
    """
    Sample a fraction of traces from a single dataframe.
    Helper function for use during parallel CSV loading.
    
    Args:
        df: Input dataframe with trace data
        fraction: Fraction of traces to sample (0.0 to 1.0)
        trace_col: Column name containing trace IDs (default: "traceid")
        random_state: Random seed for reproducibility (default: None)
    
    Returns:
        DataFrame containing all rows for the sampled traces
    """
    if fraction >= 1.0:
        return df
    
    if trace_col not in df.columns:
        return df
    
    # Get unique trace IDs
    unique_traces = df[trace_col].dropna().unique()
    
    if len(unique_traces) == 0:
        return pd.DataFrame()
    
    # Sample trace IDs
    n_samples = max(1, int(len(unique_traces) * fraction))
    sampled_trace_ids = pd.Series(unique_traces).sample(
        n=n_samples, 
        random_state=random_state
    ).values
    
    # Filter dataframe to keep all rows for sampled traces
    sampled_df = df[df[trace_col].isin(sampled_trace_ids)].copy()
    
    return sampled_df

def _read_and_sample_one(args: tuple) -> pd.DataFrame:
    """
    Read a CSV file and optionally sample traces from it.
    
    Args:
        args: Tuple of (path, sample_fraction, trace_col, random_state, read_csv_kwargs)
    
    Returns:
        Loaded and optionally sampled DataFrame
    """
    path, sample_fraction, trace_col, random_state, read_csv_kwargs = args
    
    # Read CSV
    df = pd.read_csv(path, **read_csv_kwargs)
    
    # Sample traces if requested
    if sample_fraction < 1.0:
        df = _sample_traces_from_df(df, sample_fraction, trace_col, random_state)
    
    return df

def read_csvs_parallel(
    paths: list[str | Path],
    n_workers: int | None = None,
    show_errors: bool = True,
    sample_fraction: float = 1.0,
    trace_col: str = "traceid",
    random_state: int | None = None,
    **read_csv_kwargs,
) -> pd.DataFrame:
    """
    Read multiple CSV files in parallel and optionally sample traces from each.
    
    Args:
        paths: List of CSV file paths to read
        n_workers: Number of parallel workers (default: None, uses CPU count)
        show_errors: Whether to show errors (default: True)
        sample_fraction: Fraction of traces to sample from each CSV (0.0 to 1.0, default: 1.0)
        trace_col: Column name containing trace IDs (default: "traceid")
        random_state: Random seed for reproducibility (default: None)
        **read_csv_kwargs: Additional arguments to pass to pd.read_csv
    
    Returns:
        Concatenated DataFrame from all CSVs (after sampling if requested)
    """
    paths = list(paths)
    
    # Prepare arguments for parallel processing
    process_args = [
        (path, sample_fraction, trace_col, random_state, read_csv_kwargs)
        for path in paths
    ]
    
    dfs = []

    with ProcessPoolExecutor(max_workers=n_workers) as ex:
        futures = {ex.submit(_read_and_sample_one, args): path for args, path in zip(process_args, paths)}
        for fut in tqdm(as_completed(futures), total=len(futures), desc="Reading CSVs", file=sys.stderr, dynamic_ncols=True):
            path = futures[fut]
            try:
                df = fut.result()
                dfs.append(df)
            except Exception as e:
                logger.error(f"Failed to read {path}: {e!r}")
                raise

    if not dfs:
        return pd.DataFrame()
    return pd.concat(dfs, ignore_index=True, sort=False)

# ----------------------------
# Paths & utilities
# ----------------------------

def _get_project_home() -> Path:
    """
    Determine project root directory. Works whether script is run from
    trace-analysis/ or project root.
    """
    # First, check if current working directory is the project root
    cwd = Path.cwd()
    if (cwd / "traces").exists():
        return cwd
    
    # Otherwise, derive from script location
    # Script is at trace-analysis/analyze.py, so project root is parent
    script_dir = Path(__file__).parent.resolve()
    project_root = script_dir.parent
    
    # Verify traces/ exists
    if (project_root / "traces").exists():
        return project_root
    
    # Fallback: return parent anyway (will fail later with clear error)
    return project_root

PROJECT_HOME = _get_project_home()

def get_csv_path(dataset_number: int) -> Path:
    return (
        PROJECT_HOME
        / "traces"
        / "alibaba"
        / "cluster-trace-microservices-v2022"
        / "data"
        / "CallGraph"
        / f"CallGraph_{dataset_number}.csv"
    )

# ----------------------------
# Data loading & filtering
# ----------------------------

def load_concat_datasets(
    max_dataset: int,
    max_rows: int | None = None,
    sample_fraction: float = 1.0,
    trace_col: str = "traceid",
    random_state: int | None = None,
    n_workers: int | None = None,
) -> pd.DataFrame:
    """
    Load and concatenate multiple CSV datasets in parallel, optionally sampling traces from each.
    
    Args:
        max_dataset: Maximum dataset number (0-indexed, loads datasets 0 through max_dataset)
        max_rows: Maximum number of rows to load from each CSV file (default: None, loads all)
        sample_fraction: Fraction of traces to sample from each CSV (0.0 to 1.0, default: 1.0)
        trace_col: Column name containing trace IDs (default: "traceid")
        random_state: Random seed for reproducibility (default: None)
        n_workers: Number of parallel workers (default: None, uses CPU count)
    
    Returns:
        Concatenated DataFrame from all datasets (after sampling if requested)
    """
    read_kwargs = {"on_bad_lines": "skip"}
    if max_rows is not None:
        read_kwargs["nrows"] = max_rows
    return read_csvs_parallel(
        [get_csv_path(i) for i in range(max_dataset + 1)],
        n_workers=n_workers,
        sample_fraction=sample_fraction,
        trace_col=trace_col,
        random_state=random_state,
        **read_kwargs,
    )

def _filter_chunk(args: tuple) -> pd.DataFrame:
    """
    Helper function to filter a chunk of dataframe in parallel.
    
    Args:
        args: Tuple of (chunk_df, sampled_trace_ids_set, trace_col)
    
    Returns:
        Filtered chunk dataframe
    """
    chunk_df, sampled_trace_ids_set, trace_col = args
    return chunk_df[chunk_df[trace_col].isin(sampled_trace_ids_set)].copy()

def sample_traces(
    df: pd.DataFrame, 
    fraction: float, 
    trace_col: str = "traceid", 
    random_state: int | None = None,
    n_workers: int | None = None,
    use_parallel: bool = True,
    chunk_size: int = 10000000
) -> pd.DataFrame:
    """
    Sample a fraction of traces from the dataframe.
    
    For rows belonging to the same trace, either keep them all or drop them all
    (maintains trace integrity).
    
    Args:
        df: Input dataframe with trace data
        fraction: Fraction of traces to sample (0.0 to 1.0)
        trace_col: Column name containing trace IDs (default: "traceid")
        random_state: Random seed for reproducibility (default: None)
        n_workers: Number of parallel workers for filtering (default: None, uses CPU count)
        use_parallel: Whether to use parallel filtering (default: True)
        chunk_size: Number of rows per chunk for parallel processing (default: 100000)
    
    Returns:
        DataFrame containing all rows for the sampled traces
    """
    if fraction <= 0.0 or fraction > 1.0:
        raise ValueError(f"fraction must be in (0.0, 1.0], got {fraction}")
    
    if trace_col not in df.columns:
        raise ValueError(f"Column '{trace_col}' not found in dataframe")
    
    # Get unique trace IDs
    unique_traces = df[trace_col].dropna().unique()
    
    if len(unique_traces) == 0:
        logger.warning("No valid trace IDs found")
        return pd.DataFrame()
    
    # Sample trace IDs
    n_samples = max(1, int(len(unique_traces) * fraction))
    sampled_trace_ids = pd.Series(unique_traces).sample(
        n=n_samples, 
        random_state=random_state
    ).values
    
    # Convert to set for faster lookup
    sampled_trace_ids_set = set(sampled_trace_ids)
    
    # Filter dataframe to keep all rows for sampled traces
    # Use parallel filtering for large dataframes
    if use_parallel and len(df) > chunk_size:
        # Split dataframe into chunks
        n_chunks = (len(df) + chunk_size - 1) // chunk_size
        chunks = [df.iloc[i*chunk_size:(i+1)*chunk_size].copy() for i in range(n_chunks)]
        
        logger.info(f"Filtering {len(df):,} rows in {n_chunks} chunk(s) using {n_workers or 'auto'} worker(s)")
        
        # Filter chunks in parallel
        process_args = [(chunk, sampled_trace_ids_set, trace_col) for chunk in chunks]
        
        with ProcessPoolExecutor(max_workers=n_workers) as ex:
            futures = {ex.submit(_filter_chunk, args): i for i, args in enumerate(process_args)}
            filtered_chunks = [None] * len(chunks)
            
            for fut in tqdm(as_completed(futures), total=len(futures), desc="Filtering chunks", file=sys.stderr, dynamic_ncols=True):
                chunk_idx = futures[fut]
                try:
                    filtered_chunks[chunk_idx] = fut.result()
                except Exception as e:
                    logger.error(f"Failed to filter chunk {chunk_idx}: {e!r}")
                    raise
        
        # Concatenate filtered chunks
        sampled_df = pd.concat(filtered_chunks, ignore_index=True, sort=False)
    else:
        # Use sequential filtering for small dataframes or when parallel is disabled
        sampled_df = df[df[trace_col].isin(sampled_trace_ids_set)].copy()
    
    logger.info(f"Sampled {len(sampled_trace_ids):,} traces ({fraction*100:.1f}%) from {len(unique_traces):,} unique traces")
    logger.info(f"Result: {len(sampled_df):,} rows from {len(df):,} original rows")
    
    return sampled_df

# ----------------------------
# Data cleaning
# ----------------------------

def clean_data(df: pd.DataFrame) -> pd.DataFrame:
    """
    Clean the dataframe by removing rows where 'um' or 'dm' equals 'UNKNOWN' or 'UNAVAILABLE'.
    
    Args:
        df: Input dataframe with trace data
    
    Returns:
        Cleaned dataframe
    """
    original_len = len(df)
    
    # Remove rows where um or dm equals UNKNOWN or UNAVAILABLE
    if "um" in df.columns and "dm" in df.columns:
        cleaned_df = df[
            (df["um"] != "UNKNOWN") & (df["um"] != "UNAVAILABLE") &
            (df["dm"] != "UNKNOWN") & (df["dm"] != "UNAVAILABLE")
        ].copy()
    elif "um" in df.columns:
        cleaned_df = df[(df["um"] != "UNKNOWN") & (df["um"] != "UNAVAILABLE")].copy()
    elif "dm" in df.columns:
        cleaned_df = df[(df["dm"] != "UNKNOWN") & (df["dm"] != "UNAVAILABLE")].copy()
    else:
        logger.warning("Columns 'um' or 'dm' not found, skipping cleaning")
        return df
    
    removed = original_len - len(cleaned_df)
    if removed > 0:
        logger.info(f"Cleaned data: removed {removed:,} rows where 'um' or 'dm' equals UNKNOWN or UNAVAILABLE (from {original_len:,} to {len(cleaned_df):,} rows)")
    else:
        logger.info("No rows found with 'um' or 'dm' equal to UNKNOWN or UNAVAILABLE")
    
    return cleaned_df

# ----------------------------
# Analysis functions
# ----------------------------

def _parent_rpc_id(rpc_id: str | float | int | None) -> str | None:
    """
    Return the parent RPC id for a dotted rpc_id string.
    Example: '0.1.2' -> '0.1'. Roots (no dot) return None.
    """
    if rpc_id is None:
        return None
    try:
        if isinstance(rpc_id, float) and pd.isna(rpc_id):
            return None
    except (TypeError, ValueError):
        pass
    
    rpc_str = str(rpc_id).strip()
    if not rpc_str or "." not in rpc_str:
        return None
    return rpc_str.rsplit(".", 1)[0]

def _extract_edges_from_trace(trace_df: pd.DataFrame) -> list[tuple[str, str]]:
    """
    Extract edges (um -> dm) from a single trace based on rpc_id hierarchy.
    
    Args:
        trace_df: DataFrame containing rows for a single trace
    
    Returns:
        List of (source, target) edge tuples
    """
    if "rpc_id" not in trace_df.columns or "dm" not in trace_df.columns:
        return []
    
    trace_df = trace_df.copy()
    trace_df["rpc_id_str"] = trace_df["rpc_id"].astype(str).str.strip()
    
    # Create mapping from rpc_id to dm
    rpc_to_dm = {}
    rpc_groups = trace_df.groupby("rpc_id_str")
    
    for rpc_id, group in rpc_groups:
        if rpc_id and rpc_id != "" and rpc_id != "nan":
            dm_values = group["dm"].dropna().unique()
            if len(dm_values) > 0:
                rpc_to_dm[rpc_id] = dm_values[0]
    
    # Build edge list
    edges = []
    for rpc_id, dm in rpc_to_dm.items():
        parent_rpc_id = _parent_rpc_id(rpc_id)
        
        if parent_rpc_id is None:
            # Root call - edge from USER to dm
            source = "USER"
            target = dm
        else:
            # Child call - edge from parent's dm to current dm
            if parent_rpc_id in rpc_to_dm:
                source = rpc_to_dm[parent_rpc_id]
                target = dm
            else:
                # Parent not found, skip this edge
                continue
        
        edges.append((source, target))
    
    return edges

def _hierarchical_layout(G: nx.DiGraph, ranksep: float = 2.0, nodesep: float = 0.8) -> dict:
    """
    Create a hierarchical layout for a directed graph with roots at the top.
    Uses Graphviz's 'dot' layout if available (same as analyze.py), otherwise falls back
    to a custom hierarchical layout.
    
    Args:
        G: NetworkX directed graph
        ranksep: Minimum distance between ranks/layers (for Graphviz)
        nodesep: Minimum distance between nodes in the same rank (for Graphviz)
    
    Returns:
        Dictionary mapping nodes to (x, y) positions
    """
    if G.number_of_nodes() == 0:
        return {}
    
    # Try to use Graphviz layout (same as analyze.py)
    try:
        pos = nx.nx_agraph.graphviz_layout(
            G, prog="dot", args=f"-Granksep={ranksep} -Gnodesep={nodesep}"
        )
        return pos
    except (ImportError, AttributeError, Exception):
        # Fall back to custom hierarchical layout if Graphviz is not available
        pass
    
    # Custom hierarchical layout (fallback)
    # Find root nodes (nodes with in_degree == 0)
    roots = [n for n in G.nodes() if G.in_degree(n) == 0]
    
    if not roots:
        # If no roots found, use nodes with minimum in_degree
        min_in_degree = min(G.in_degree(n) for n in G.nodes())
        roots = [n for n in G.nodes() if G.in_degree(n) == min_in_degree]
    
    # Compute depth/layer for each node using BFS
    node_depth = {}
    visited = set()
    queue = [(root, 0) for root in roots]
    
    while queue:
        node, depth = queue.pop(0)
        if node in visited:
            continue
        visited.add(node)
        node_depth[node] = depth
        
        # Add children to queue
        for successor in G.successors(node):
            if successor not in visited:
                queue.append((successor, depth + 1))
    
    # Handle any unvisited nodes (disconnected components)
    for node in G.nodes():
        if node not in node_depth:
            # Find shortest path to any root
            min_depth = float('inf')
            for root in roots:
                try:
                    path_length = nx.shortest_path_length(G, root, node)
                    min_depth = min(min_depth, path_length)
                except nx.NetworkXNoPath:
                    continue
            node_depth[node] = min_depth if min_depth != float('inf') else 0
    
    # Group nodes by depth
    depth_groups = {}
    for node, depth in node_depth.items():
        if depth not in depth_groups:
            depth_groups[depth] = []
        depth_groups[depth].append(node)
    
    max_depth = max(depth_groups.keys()) if depth_groups else 0
    
    # Position nodes: roots at top (y=0), children below (y increases downward)
    pos = {}
    for depth, nodes in depth_groups.items():
        # Y position: top is 0, bottom is max_depth (inverted for matplotlib)
        y = max_depth - depth
        
        # X positions: distribute nodes evenly across width
        n_nodes = len(nodes)
        if n_nodes == 1:
            x_positions = [0.0]
        else:
            x_positions = [i / (n_nodes - 1) * 2 - 1 for i in range(n_nodes)]
        
        for node, x in zip(sorted(nodes), x_positions):
            pos[node] = (x, y)
    
    return pos

def _compute_dynamic_figsize(
    G: nx.DiGraph,
    base_width: float = 14.0,
    base_height: float = 10.0,
    width_per_node: float = 0.4,
    height_per_node: float = 0.2,
    min_width: float = 14.0,
    min_height: float = 10.0,
    max_width: float = 80.0,
    max_height: float = 50.0,
) -> tuple[float, float]:
    """
    Compute dynamic figure size based on graph complexity.
    
    Args:
        G: NetworkX graph
        base_width, base_height: Base dimensions
        width_per_node, height_per_node: Scaling factors per node
        min_width, min_height: Minimum dimensions
        max_width, max_height: Maximum dimensions
    
    Returns:
        (width, height) tuple for matplotlib figsize
    """
    num_nodes = G.number_of_nodes()
    num_edges = G.number_of_edges()
    
    # Base calculation on number of nodes
    width = base_width + (num_nodes * width_per_node)
    height = base_height + (num_nodes * height_per_node)
    
    # Add extra space for highly connected graphs
    if num_nodes > 0:
        edge_density = num_edges / num_nodes
        if edge_density > 5:
            width *= 1.1
            height *= 1.05
    
    # Clamp to min/max bounds
    width = max(min_width, min(width, max_width))
    height = max(min_height, min(height, max_height))
    
    return (width, height)

def _slugify(name: str) -> str:
    """
    Convert a service name to a filesystem-safe slug.
    
    Args:
        name: Service name string
    
    Returns:
        Filesystem-safe slug
    """
    s = re.sub(r"[^\w\-]+", "_", name.strip())
    s = re.sub(r"_+", "_", s).strip("_")
    return s or "service"

def _extract_timing_from_trace(
    trace_df: pd.DataFrame,
    G: nx.DiGraph,
) -> dict[tuple[str, str], list[dict]]:
    """
    Extract timing information for parent-child relationships from a trace.
    
    Args:
        trace_df: DataFrame containing rows for a single trace
        G: NetworkX graph with the service's call structure
    
    Returns:
        Dictionary mapping (parent_dm, child_dm) -> list of timing dicts
        Each timing dict has: start_time, end_time, parent_start, parent_end, rpc_id
    """
    if "rpc_id" not in trace_df.columns or "dm" not in trace_df.columns:
        return {}
    
    if "timestamp" not in trace_df.columns or "rt" not in trace_df.columns:
        return {}
    
    trace_df = trace_df.copy()
    trace_df["rpc_id_str"] = trace_df["rpc_id"].astype(str).str.strip()
    
    # Convert timestamps and runtime
    trace_df["timestamp"] = pd.to_numeric(trace_df["timestamp"], errors="coerce")
    trace_df["rt"] = pd.to_numeric(trace_df["rt"], errors="coerce")
    trace_df["end_time"] = trace_df["timestamp"] + trace_df["rt"]
    
    # Create mapping from rpc_id to (dm, start_time, end_time)
    rpc_info = {}
    rpc_groups = trace_df.groupby("rpc_id_str")
    
    for rpc_id, group in rpc_groups:
        if rpc_id and rpc_id != "" and rpc_id != "nan":
            dm_values = group["dm"].dropna().unique()
            if len(dm_values) > 0:
                dm = dm_values[0]
                # Use min start and max end to handle multiple rows with same rpc_id
                # Try to get valid timestamps - use median if min/max are invalid
                valid_timestamps = group["timestamp"].dropna()
                valid_end_times = group["end_time"].dropna()
                
                if len(valid_timestamps) > 0 and len(valid_end_times) > 0:
                    start_time = valid_timestamps.min()
                    end_time = valid_end_times.max()
                    # Only include if we have reasonable timing data
                    if pd.notna(start_time) and pd.notna(end_time) and end_time >= start_time:
                        rpc_info[rpc_id] = {
                            "dm": dm,
                            "start_time": start_time,
                            "end_time": end_time,
                        }
    
    # Build parent-child timing relationships
    timing_data = defaultdict(list)
    
    # Also create a mapping from rpc_id to dm for edges without timing
    rpc_to_dm_no_timing = {}
    for rpc_id, group in trace_df.groupby("rpc_id_str"):
        if rpc_id and rpc_id != "" and rpc_id != "nan":
            dm_values = group["dm"].dropna().unique()
            if len(dm_values) > 0 and rpc_id not in rpc_info:
                # This rpc_id exists but has no valid timing data
                rpc_to_dm_no_timing[rpc_id] = dm_values[0]
    
    for rpc_id, info in rpc_info.items():
        parent_rpc_id = _parent_rpc_id(rpc_id)
        
        if parent_rpc_id is None:
            # Root call - parent is USER
            if "USER" in G.nodes():
                parent_dm = "USER"
                child_dm = info["dm"]
                if (parent_dm, child_dm) in G.edges():
                    timing_data[(parent_dm, child_dm)].append({
                        "start_time": info["start_time"],
                        "end_time": info["end_time"],
                        "parent_start": info["start_time"],  # USER call starts when child starts
                        "parent_end": info["end_time"],
                        "rpc_id": rpc_id,
                    })
        else:
            # Child call - find parent's timing
            if parent_rpc_id in rpc_info:
                # Both parent and child have valid timing
                parent_info = rpc_info[parent_rpc_id]
                parent_dm = parent_info["dm"]
                child_dm = info["dm"]
                
                # Only include if this edge exists in the graph
                if (parent_dm, child_dm) in G.edges():
                    timing_data[(parent_dm, child_dm)].append({
                        "start_time": info["start_time"],
                        "end_time": info["end_time"],
                        "parent_start": parent_info["start_time"],
                        "parent_end": parent_info["end_time"],
                        "rpc_id": rpc_id,
                    })
            elif parent_rpc_id in rpc_to_dm_no_timing:
                # Child has timing but parent doesn't - still try to include child timing
                # Use child's timing as a fallback for parent timing
                parent_dm = rpc_to_dm_no_timing[parent_rpc_id]
                child_dm = info["dm"]
                
                if (parent_dm, child_dm) in G.edges():
                    # Use child's start as parent start estimate
                    timing_data[(parent_dm, child_dm)].append({
                        "start_time": info["start_time"],
                        "end_time": info["end_time"],
                        "parent_start": info["start_time"],  # Estimate: parent starts when child starts
                        "parent_end": info["end_time"],  # Estimate: parent ends when child ends
                        "rpc_id": rpc_id,
                    })
    
    return dict(timing_data)

def _classify_call_pattern(
    child_timings_dict: dict[str, list[dict]],
    overlap_threshold: float = 0.1,
) -> tuple[str, float]:
    """
    Classify whether a parent calls its children sequentially, in parallel, or mixed.
    Compares all children of the same parent together.
    
    Args:
        child_timings_dict: Dictionary mapping child_dm -> list of timing dicts
        overlap_threshold: Fraction of overlap needed to consider calls parallel (default: 0.1)
    
    Returns:
        Tuple of (pattern_type, parallel_ratio)
        pattern_type: "sequential", "parallel", or "mixed"
        parallel_ratio: Fraction of sibling pairs that overlap (0.0 to 1.0)
    """
    if len(child_timings_dict) < 2:
        return ("sequential", 0.0)
    
    # Group timings by parent call instance (same parent_start/end)
    parent_groups = defaultdict(lambda: defaultdict(list))
    for child_dm, timings in child_timings_dict.items():
        for timing in timings:
            parent_key = (timing["parent_start"], timing["parent_end"])
            parent_groups[parent_key][child_dm].append(timing)
    
    if not parent_groups:
        return ("sequential", 0.0)
    
    parallel_count = 0
    total_pairs = 0
    
    # For each parent call instance, check if children overlap
    for parent_key, children_timings in parent_groups.items():
        if len(children_timings) < 2:
            continue
        
        # Get the earliest start and latest end for each child in this parent call
        child_ranges = {}
        for child_dm, timings in children_timings.items():
            if timings:
                # Prefer original keys (start_time/end_time) for classification, fall back to normalized (start/end)
                if "start_time" in timings[0]:
                    starts = [t["start_time"] for t in timings]
                    ends = [t["end_time"] for t in timings]
                elif "start" in timings[0]:
                    starts = [t["start"] for t in timings]
                    ends = [t["end"] for t in timings]
                else:
                    continue  # Skip if no timing data
                child_ranges[child_dm] = (min(starts), max(ends))
        
        # Compare all pairs of children
        child_list = list(child_ranges.items())
        for i in range(len(child_list)):
            for j in range(i + 1, len(child_list)):
                child1_dm, (start1, end1) = child_list[i]
                child2_dm, (start2, end2) = child_list[j]
                
                total_pairs += 1
                
                # Check if the two children overlap in time
                overlap_start = max(start1, start2)
                overlap_end = min(end1, end2)
                
                if overlap_start < overlap_end:
                    # They overlap - calculate overlap ratio
                    overlap_duration = overlap_end - overlap_start
                    min_duration = min(end1 - start1, end2 - start2)
                    if min_duration > 0:
                        overlap_ratio = overlap_duration / min_duration
                        if overlap_ratio >= overlap_threshold:
                            parallel_count += 1
    
    if total_pairs == 0:
        return ("sequential", 0.0)
    
    parallel_ratio = parallel_count / total_pairs
    
    if parallel_ratio >= 0.7:
        return ("parallel", parallel_ratio)
    elif parallel_ratio <= 0.3:
        return ("sequential", parallel_ratio)
    else:
        return ("mixed", parallel_ratio)

def _draw_timeline_graph(
    G: nx.DiGraph,
    timing_data: dict[tuple[str, str], list[dict]],
    title: str,
    output_path: Path,
    num_nodes: int,
) -> None:
    """
    Draw a timeline graph showing parent-child call patterns over time.
    The layout mirrors the DAG structure with time on x-axis and hierarchy on y-axis.
    
    Args:
        G: NetworkX directed graph (USER subgraph)
        timing_data: Dictionary mapping (parent, child) -> list of timing dicts
        title: Title for the graph
        output_path: Path to save the image
        num_nodes: Number of nodes in the graph (for sizing)
    """
    if G.number_of_nodes() == 0:
        return
    
    # Get hierarchical layout positions
    pos = _hierarchical_layout(G)
    if not pos:
        return
    
    # Compute dynamic figure size
    if num_nodes < 50:
        figsize = (20.0, max(12.0, num_nodes * 0.3))
    elif num_nodes < 200:
        figsize = (24.0, max(14.0, num_nodes * 0.2))
    else:
        figsize = (28.0, max(16.0, num_nodes * 0.15))
    
    fig, (ax_left, ax_right) = plt.subplots(1, 2, figsize=figsize, 
                                           gridspec_kw={'width_ratios': [1, 2]})
    
    # Left panel: DAG structure (mirror of user graph)
    # Draw nodes and edges
    for node, (x, y) in pos.items():
        ax_left.scatter(x, y, s=500, c='lightblue', edgecolors='black', zorder=3)
        ax_left.text(x, y, node, fontsize=7, ha='center', va='center', fontweight='bold')
    
    # Draw edges
    for u, v in G.edges():
        if u in pos and v in pos:
            x1, y1 = pos[u]
            x2, y2 = pos[v]
            ax_left.plot([x1, x2], [y1, y2], 'k-', linewidth=1, alpha=0.3, zorder=1)
    
    ax_left.set_title('Call Graph Structure', fontsize=10, fontweight='bold')
    ax_left.axis('off')
    
    # Right panel: Timeline visualization
    # First, classify patterns using original timings
    # Then normalize for visualization
    edge_timelines = {}
    
    for (parent, child), timings in timing_data.items():
        if not timings or (parent, child) not in G.edges():
            continue
        
        # Normalize timestamps relative to parent start for each trace
        normalized_timings = []
        for timing in timings:
            parent_start = timing["parent_start"]
            relative_start = timing["start_time"] - parent_start
            relative_end = timing["end_time"] - parent_start
            normalized_timings.append({
                "start": relative_start,
                "end": relative_end,
                "start_time": timing["start_time"],  # Keep original for classification
                "end_time": timing["end_time"],  # Keep original for classification
                "parent_start": timing["parent_start"],  # Keep original for grouping
                "parent_end": timing["parent_end"],  # Keep original for grouping
            })
        
        edge_timelines[(parent, child)] = normalized_timings
    
    if not edge_timelines:
        plt.close()
        return
    
    # Determine time range for x-axis (use 95th percentile to avoid outliers)
    all_times = []
    for timings in edge_timelines.values():
        for timing in timings:
            all_times.append(timing["end"])
            all_times.append(timing["parent_end"])
    
    if not all_times:
        plt.close()
        return
    
    max_time = np.percentile(all_times, 95) if len(all_times) > 0 else max(all_times)
    if max_time == 0:
        max_time = 1.0
    
    # Map nodes to y-positions based on hierarchy (for vertical layout)
    node_y_positions = {}
    y_positions = sorted(set(y for x, y in pos.values()), reverse=True)
    y_to_level = {y: i for i, y in enumerate(y_positions)}
    
    for node, (x, y) in pos.items():
        level = y_to_level[y]
        node_y_positions[node] = level
    
    max_level = len(y_to_level) - 1
    
    # Calculate hierarchy depth for each node (for horizontal positioning)
    # Depth = distance from USER (or root)
    node_depths = {}
    if "USER" in G.nodes():
        # BFS from USER to calculate depths
        queue = [("USER", 0)]
        visited = set()
        while queue:
            node, depth = queue.pop(0)
            if node in visited:
                continue
            visited.add(node)
            node_depths[node] = depth
            for child in G.successors(node):
                if child not in visited:
                    queue.append((child, depth + 1))
        # Handle any unvisited nodes (shouldn't happen in USER subgraph, but be safe)
        for node in G.nodes():
            if node not in node_depths:
                node_depths[node] = 0
    else:
        # No USER node, use in-degree to estimate depth
        for node in G.nodes():
            node_depths[node] = 0
    
    max_depth = max(node_depths.values()) if node_depths else 0
    
    # Colors for different patterns
    colors = {'sequential': '#2E86AB', 'parallel': '#A23B72', 'mixed': '#F18F01'}
    
    # Calculate column width for hierarchy levels
    # Each depth level gets a column, with time flowing horizontally within
    column_width = max_time * 1.2  # Extra space for labels
    
    # Calculate max visualization depth (may be deeper than BFS depth due to parent+1 placement)
    max_vis_depth = max_depth
    for parent in G.nodes():
        if parent in node_depths:
            parent_depth = node_depths[parent]
            children = list(G.successors(parent))
            if children:
                # Children will be at parent_depth + 1
                max_vis_depth = max(max_vis_depth, parent_depth + 1)
    
    total_width = (max_vis_depth + 1) * column_width
    
    # Track which (node, x_pos) combinations have been labeled to avoid duplicates
    labeled_positions = set()
    
    # First, draw all node labels at their base hierarchy depth
    for node in G.nodes():
        if node not in node_y_positions:
            continue
        level = node_y_positions[node]
        depth = node_depths.get(node, 0)
        x_pos = depth * column_width
        pos_key = (node, x_pos, level)
        
        if pos_key not in labeled_positions:
            # Draw node label at its depth column
            ax_right.text(x_pos - 0.05 * max_time, level, node, 
                         fontsize=8, ha='right', va='center', fontweight='bold',
                         bbox=dict(boxstyle='round,pad=0.3', facecolor='lightgray', alpha=0.5))
            labeled_positions.add(pos_key)
    
    # For each parent node, show when it calls its children
    for parent in sorted(G.nodes(), key=lambda n: node_y_positions.get(n, 0)):
        if parent not in node_y_positions:
            continue
        
        parent_level = node_y_positions[parent]
        parent_depth = node_depths.get(parent, 0)
        parent_x = parent_depth * column_width
        
        children = sorted([child for child in G.successors(parent) if child in node_y_positions])
        
        if not children:
            # Leaf node - already labeled above, skip timeline bars
            continue
        
        # Get timing data for all children of this parent
        child_timings = {}
        for child in children:
            edge = (parent, child)
            if edge in edge_timelines:
                child_timings[child] = edge_timelines[edge]
        
        # Classify pattern for this parent's children (if we have timing data)
        if child_timings:
            pattern_type, parallel_ratio = _classify_call_pattern(child_timings)
            color = colors.get(pattern_type, 'gray')
        else:
            # No timing data - use gray
            color = 'gray'
        
        # Draw parent node label (overwrite the gray one)
        parent_pos_key = (parent, parent_x, parent_level)
        ax_right.text(parent_x - 0.05 * max_time, parent_level, parent, 
                     fontsize=8, ha='right', va='center', fontweight='bold',
                     bbox=dict(boxstyle='round,pad=0.3', facecolor='lightblue', alpha=0.7))
        labeled_positions.add(parent_pos_key)
        
        # Draw timeline bars for each child
        # Children appear in the next column to the right of their parent
        for i, child in enumerate(children):
            edge = (parent, child)
            child_level = node_y_positions[child]
            # For visualization, place child one column to the right of parent
            # This ensures the DAG structure is clear: children are always to the right
            child_vis_depth = parent_depth + 1
            child_x = child_vis_depth * column_width
            
            # Calculate y position with slight offset for multiple children at same level
            y_offset = (i - len(children) / 2 + 0.5) * 0.12
            y_pos = child_level + y_offset
            
            if edge in child_timings and child_timings[edge]:
                timings = child_timings[edge]
                
                # Use median start and end times for visualization
                starts = [t["start"] for t in timings]
                ends = [t["end"] for t in timings]
                median_start = np.median(starts)
                median_end = np.median(ends)
                
                # Draw bar at child's column position
                width = max(0.005 * max_time, median_end - median_start)
                ax_right.barh(y_pos, width, left=child_x + median_start, height=0.15, 
                             color=color, alpha=0.7, edgecolor='black', linewidth=0.5)
                
                # Draw connecting line from parent to child (horizontal then vertical)
                # Make edges thicker and more prominent to show dependency structure
                # Horizontal line from parent to start of child column
                ax_right.plot([parent_x, child_x], [parent_level, parent_level], 
                             color='black', linestyle='--', linewidth=2.0, alpha=0.6, zorder=1)
                # Vertical line down to child
                ax_right.plot([child_x, child_x], [parent_level, y_pos], 
                             color='black', linestyle='--', linewidth=2.0, alpha=0.6, zorder=1)
                # Horizontal line to child's timeline bar
                ax_right.plot([child_x, child_x + median_start], [y_pos, y_pos], 
                             color='black', linestyle='--', linewidth=2.0, alpha=0.6, zorder=1)
                
                # Label child at its column (only if not already labeled at this position)
                child_pos_key = (child, child_x, y_pos)
                if child_pos_key not in labeled_positions:
                    ax_right.text(child_x - 0.02 * max_time, y_pos, child,
                                fontsize=7, ha='right', va='center', fontstyle='italic')
                    labeled_positions.add(child_pos_key)
            else:
                # No timing data - draw a placeholder at child's column
                placeholder_start = child_x + 0.1 * max_time
                placeholder_width = 0.05 * max_time
                placeholder_center = placeholder_start + placeholder_width / 2
                ax_right.barh(y_pos, placeholder_width, left=placeholder_start, height=0.15, 
                             color='lightgray', alpha=0.3, edgecolor='gray', linewidth=0.5)
                
                # Draw connecting line from parent to child (thinner for no-data case)
                ax_right.plot([parent_x, child_x], [parent_level, parent_level], 
                             color='gray', linestyle=':', linewidth=1.5, alpha=0.4, zorder=1)
                ax_right.plot([child_x, child_x], [parent_level, y_pos], 
                             color='gray', linestyle=':', linewidth=1.5, alpha=0.4, zorder=1)
                ax_right.plot([child_x, placeholder_start], [y_pos, y_pos], 
                             color='gray', linestyle=':', linewidth=1.5, alpha=0.4, zorder=1)
                
                # Label child inside the placeholder bar (only if not already labeled at this position)
                child_pos_key = (child, child_x, y_pos)
                if child_pos_key not in labeled_positions:
                    ax_right.text(placeholder_center, y_pos, child + " (no data)",
                                fontsize=6, ha='center', va='center', fontstyle='italic', 
                                color='black', alpha=0.8, weight='bold',
                                bbox=dict(boxstyle='round,pad=0.2', facecolor='white', alpha=0.7, edgecolor='none'))
                    labeled_positions.add(child_pos_key)
    
    # Set axis properties
    # X-axis: hierarchy depth (columns) with time flowing within each column
    # Y-axis: hierarchy level (vertical position)
    ax_right.set_xlim(-0.1 * max_time, total_width + 0.1 * max_time)
    ax_right.set_ylim(-0.5, max_level + 0.5)
    ax_right.set_xlabel('Hierarchy Depth (columns) → Time (within each column, normalized to parent start)', 
                       fontsize=9)
    ax_right.set_ylabel('Hierarchy Level (from USER)', fontsize=10)
    ax_right.set_title('Call Pattern Timeline (Children appear to the right of their parent)', 
                     fontsize=10, fontweight='bold')
    ax_right.grid(True, alpha=0.3, axis='both')
    
    # Add vertical lines to separate hierarchy depth columns
    for depth in range(max_vis_depth + 2):
        x_pos = depth * column_width
        ax_right.axvline(x=x_pos, color='lightgray', linestyle='--', linewidth=0.5, alpha=0.3)
    
    # Add legend
    legend_elements = [
        mpatches.Patch(facecolor=colors['sequential'], label='Sequential', alpha=0.7),
        mpatches.Patch(facecolor=colors['parallel'], label='Parallel', alpha=0.7),
        mpatches.Patch(facecolor=colors['mixed'], label='Mixed', alpha=0.7),
    ]
    ax_right.legend(handles=legend_elements, loc='upper right', fontsize=8)
    
    plt.suptitle(title, fontsize=12, fontweight='bold', y=0.98)
    plt.tight_layout(rect=[0, 0, 1, 0.96])
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()

def reachable_subgraph(G: nx.DiGraph, source: str = "USER") -> nx.DiGraph:
    """
    Extract the subgraph reachable from a source node.
    
    Args:
        G: NetworkX directed graph
        source: Source node name (default: "USER")
    
    Returns:
        Subgraph containing source and all nodes reachable from it
    """
    if source not in G:
        # Return empty graph if source not found
        return G.__class__()
    
    reachable = {source} | nx.descendants(G, source)
    view = G.subgraph(reachable)
    
    H = G.__class__()
    H.graph.update(copy.deepcopy(G.graph))
    H.add_nodes_from((n, copy.deepcopy(view.nodes[n])) for n in view.nodes)
    
    if G.is_multigraph():
        H.add_edges_from(
            (u, v, k, copy.deepcopy(view.get_edge_data(u, v, k)))
            for u, v, k in view.edges(keys=True)
        )
    else:
        H.add_edges_from(
            (u, v, copy.deepcopy(view.get_edge_data(u, v)))
            for u, v in view.edges()
        )
    return H

def _draw_graph(
    G: nx.DiGraph,
    title: str,
    output_path: Path,
    num_nodes: int,
) -> None:
    """
    Draw a graph with dynamic sizing and save to file.
    
    Args:
        G: NetworkX directed graph to draw
        title: Title for the graph
        output_path: Path to save the image
        num_nodes: Number of nodes in the graph (for sizing)
    """
    if G.number_of_nodes() == 0:
        return
    
    # Compute dynamic figure size based on graph complexity
    if num_nodes < 50:
        figsize = _compute_dynamic_figsize(
            G,
            base_width=14.0,
            base_height=10.0,
            width_per_node=0.4,
            height_per_node=0.2,
            min_width=14.0,
            min_height=10.0,
            max_width=40.0,
            max_height=30.0,
        )
    elif num_nodes < 200:
        figsize = _compute_dynamic_figsize(
            G,
            base_width=16.0,
            base_height=12.0,
            width_per_node=0.25,
            height_per_node=0.15,
            min_width=16.0,
            min_height=12.0,
            max_width=50.0,
            max_height=35.0,
        )
    else:
        figsize = _compute_dynamic_figsize(
            G,
            base_width=18.0,
            base_height=14.0,
            width_per_node=0.15,
            height_per_node=0.1,
            min_width=18.0,
            min_height=14.0,
            max_width=60.0,
            max_height=40.0,
        )
    
    # Auto-compute node_size, font_size, and spacing based on graph complexity
    if num_nodes < 20:
        node_size = 3000
        font_size = 10
        ranksep = 2.0
        nodesep = 0.8
    elif num_nodes < 50:
        node_size = 2800
        font_size = 9
        ranksep = 1.8
        nodesep = 0.7
    elif num_nodes < 100:
        node_size = 2600
        font_size = 8
        ranksep = 1.5
        nodesep = 0.6
    elif num_nodes < 200:
        node_size = 2400
        font_size = 7
        ranksep = 1.2
        nodesep = 0.5
    else:
        node_size = 2200
        font_size = 7
        ranksep = 1.0
        nodesep = 0.4
    
    plt.figure(figsize=figsize)
    pos = _hierarchical_layout(G, ranksep=ranksep, nodesep=nodesep)
    
    # Get edge weights for visualization
    edge_weights = [G[u][v].get('weight', 1) for u, v in G.edges()]
    max_weight = max(edge_weights) if edge_weights else 1
    min_weight = min(edge_weights) if edge_weights else 1
    
    # Normalize edge widths (min 1, max 5)
    edge_widths = [1 + 4 * (w - min_weight) / (max_weight - min_weight) if max_weight > min_weight else 3 
                   for w in edge_weights]
    
    # Draw nodes
    nx.draw_networkx_nodes(G, pos, node_color='lightblue', node_size=node_size, alpha=0.9)
    
    # Draw edges with varying widths based on frequency
    nx.draw_networkx_edges(
        G, pos, 
        edge_color='gray', 
        arrows=True, 
        arrowsize=20, 
        alpha=0.6,
        width=edge_widths
    )
    
    # Draw labels
    nx.draw_networkx_labels(G, pos, font_size=font_size, font_weight='bold')
    
    # Add edge labels with frequencies
    edge_labels = {(u, v): str(G[u][v].get('weight', 1)) for u, v in G.edges()}
    edge_label_font_size = max(6, font_size - 2)
    nx.draw_networkx_edge_labels(G, pos, edge_labels, font_size=edge_label_font_size)
    
    plt.title(title, fontsize=16, fontweight='bold')
    plt.axis('off')
    plt.tight_layout()
    
    # Save graph
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()

def _compute_graph_statistics(G: nx.DiGraph) -> dict:
    """
    Compute statistics for a directed graph.
    
    Args:
        G: NetworkX directed graph
    
    Returns:
        Dictionary with statistics: num_nodes, out_degree_avg, out_degree_min, out_degree_max,
        in_degree_avg, in_degree_min, in_degree_max
    """
    if G.number_of_nodes() == 0:
        return {
            "num_nodes": 0,
            "out_degree_avg": 0.0,
            "out_degree_min": 0,
            "out_degree_max": 0,
            "in_degree_avg": 0.0,
            "in_degree_min": 0,
            "in_degree_max": 0,
        }
    
    out_degrees = [d for n, d in G.out_degree()]
    in_degrees = [d for n, d in G.in_degree()]
    
    return {
        "num_nodes": G.number_of_nodes(),
        "out_degree_avg": sum(out_degrees) / len(out_degrees) if out_degrees else 0.0,
        "out_degree_min": min(out_degrees) if out_degrees else 0,
        "out_degree_max": max(out_degrees) if out_degrees else 0,
        "in_degree_avg": sum(in_degrees) / len(in_degrees) if in_degrees else 0.0,
        "in_degree_min": min(in_degrees) if in_degrees else 0,
        "in_degree_max": max(in_degrees) if in_degrees else 0,
    }

def _process_service(
    service_name: str,
    service_df: pd.DataFrame,
    trace_col: str,
    graphs_dir: Path,
) -> tuple[str, nx.DiGraph, dict, bool]:
    """
    Process a single service: extract edges, create graph, and save visualization.
    
    Args:
        service_name: Name of the service
        service_df: DataFrame containing rows for this service
        trace_col: Column name containing trace IDs
        graphs_dir: Directory to save graph visualizations
    
    Returns:
        Tuple of (service_name, graph, statistics_dict, has_user_root, user_subgraph_sufficient)
        where has_user_root is True if the graph has 'USER' as a root node,
        and user_subgraph_sufficient is True if the USER-reachable subgraph has at least 5 nodes
    """
    # Get unique traces for this service
    unique_traces = service_df[trace_col].dropna().unique()
    
    if len(unique_traces) == 0:
        logger.warning(f"No valid traces found for service {service_name}")
        G = nx.DiGraph()
        stats = _compute_graph_statistics(G)
        stats["num_traces"] = 0
        return (service_name, G, stats, False, False)
    
    # Aggregate edges across all traces
    edge_counter: dict[tuple[str, str], int] = {}
    
    for trace_id in unique_traces:
        trace_df = service_df[service_df[trace_col] == trace_id]
        edges = _extract_edges_from_trace(trace_df)
        
        # Count edge frequencies
        for edge in edges:
            edge_counter[edge] = edge_counter.get(edge, 0) + 1
    
    # Create graph with edge weights
    G = nx.DiGraph()
    for (source, target), frequency in edge_counter.items():
        G.add_edge(source, target, weight=frequency)
    
    # Check if graph has 'USER' as a root node (in_degree == 0)
    has_user_root = "USER" in G.nodes() and G.in_degree("USER") == 0
    
    # Check if USER subgraph has at least 5 nodes
    user_subgraph_sufficient = False
    if has_user_root:
        G_user = reachable_subgraph(G, source="USER")
        user_subgraph_sufficient = G_user.number_of_nodes() >= 5
    
    # Compute statistics
    stats = _compute_graph_statistics(G)
    # Add trace count to statistics
    stats["num_traces"] = len(unique_traces)
    
    # Only create visualizations if graph has USER as a root node and USER subgraph has >= 5 nodes
    if G.number_of_nodes() > 0 and has_user_root and user_subgraph_sufficient:
        # Create service-specific directory
        service_slug = _slugify(service_name)
        service_dir = graphs_dir / service_slug
        service_dir.mkdir(parents=True, exist_ok=True)
        
        # Draw full graph (all nodes)
        num_nodes = G.number_of_nodes()
        output_path = service_dir / "graph_all_nodes.png"
        title = f"Aggregated Call Graph for Service '{service_name}' (All Nodes)\n(Edge thickness and labels indicate frequency)"
        _draw_graph(G, title, output_path, num_nodes)
        
        # Draw USER-reachable subgraph
        G_user = reachable_subgraph(G, source="USER")
        if G_user.number_of_nodes() > 0:
            num_nodes_user = G_user.number_of_nodes()
            output_path = service_dir / "graph_user.png"
            title = f"Aggregated Call Graph for Service '{service_name}' (USER-Reachable Subgraph)\n(Edge thickness and labels indicate frequency)"
            _draw_graph(G_user, title, output_path, num_nodes_user)
            
            # Extract and aggregate timing data for timeline visualization
            if "timestamp" in service_df.columns and "rt" in service_df.columns:
                # Aggregate timing data across all traces
                aggregated_timing: dict[tuple[str, str], list[dict]] = defaultdict(list)
                
                for trace_id in unique_traces:
                    trace_df = service_df[service_df[trace_col] == trace_id]
                    trace_timing = _extract_timing_from_trace(trace_df, G_user)
                    
                    # Merge into aggregated timing
                    for edge, timings in trace_timing.items():
                        if edge in G_user.edges():  # Only include edges in USER subgraph
                            aggregated_timing[edge].extend(timings)
                
                # Log edges in graph but without timing data for debugging
                edges_without_timing = set(G_user.edges()) - set(aggregated_timing.keys())
                if edges_without_timing:
                    logger.debug(f"Service {service_name}: {len(edges_without_timing)} edges in graph but no timing data: {list(edges_without_timing)[:5]}...")
                
                # Draw timeline graph (even if no timing data, to show structure)
                output_path = service_dir / "pattern_timeline.png"
                title = f"Call Pattern Timeline for Service '{service_name}'\n(Shows sequential vs parallel call patterns)"
                _draw_timeline_graph(G_user, aggregated_timing, title, output_path, num_nodes_user)
    
    # Return both flags separately so we can track rejection reasons
    return (service_name, G, stats, has_user_root, user_subgraph_sufficient)

def _process_service_wrapper(args: tuple) -> tuple[str, nx.DiGraph, dict, bool, bool]:
    """
    Wrapper function for parallel processing of services.
    Extracts arguments from tuple for ProcessPoolExecutor compatibility.
    
    Args:
        args: Tuple of (service_name, service_df, trace_col, graphs_dir_str)
              where graphs_dir_str is a string path that will be converted to Path
    
    Returns:
        Tuple of (service_name, graph, statistics_dict, has_user_root, user_subgraph_sufficient)
    """
    service_name, service_df, trace_col, graphs_dir_str = args
    graphs_dir = Path(graphs_dir_str)
    return _process_service(service_name, service_df, trace_col, graphs_dir)

def analyze_call_graphs(df: pd.DataFrame, trace_col: str = "traceid", top_n: int = 100, n_workers: int | None = None) -> None:
    """
    Analyze call graphs by grouping by service and aggregating edges across all traces.
    For each service, compute the union of all edges and their frequencies, then plot.
    Only processes the top N services by trace count.
    Print statistics sorted by number of nodes (descending).
    
    Args:
        df: Input dataframe with trace data
        trace_col: Column name containing trace IDs (default: "traceid")
        top_n: Number of top services by trace count to process (default: 100)
    """
    # Check required columns
    required_cols = ["service", trace_col, "rpc_id", "um", "dm"]
    missing_cols = [col for col in required_cols if col not in df.columns]
    if missing_cols:
        logger.warning(f"Missing required columns: {missing_cols}. Available columns: {list(df.columns)}")
        return
    
    # Create graphs directory
    graphs_dir = Path(__file__).parent / "graphs"
    graphs_dir.mkdir(exist_ok=True)
    logger.info(f"Graphs will be saved to: {graphs_dir}")
    
    # Group by service
    logger.info(f"{'='*80}")
    logger.info("Grouping dataset by service")
    logger.info(f"{'='*80}")
    
    service_groups = df.groupby("service")
    service_names = list(service_groups.groups.keys())
    
    if len(service_names) == 0:
        logger.warning("No services found in dataset")
        return
    
    logger.info(f"Found {len(service_names)} service(s)")
    
    # Count traces per service and select top N
    logger.info(f"Counting traces per service...")
    service_trace_counts = []
    for service_name in service_names:
        service_df = service_groups.get_group(service_name)
        unique_traces = service_df[trace_col].dropna().unique()
        trace_count = len(unique_traces)
        service_trace_counts.append((service_name, trace_count))
    
    # Sort by trace count (descending) and take top N
    service_trace_counts.sort(key=lambda x: x[1], reverse=True)
    top_services = [name for name, _ in service_trace_counts[:top_n]]
    
    logger.info(f"Selecting top {min(top_n, len(service_names))} service(s) by trace count")
    logger.info(f"Top service: {top_services[0]} with {service_trace_counts[0][1]:,} traces")
    if len(top_services) > 1:
        logger.info(f"Bottom selected service: {top_services[-1]} with {service_trace_counts[min(top_n, len(service_names))-1][1]:,} traces")
    
    # Process each selected service in parallel
    all_stats = []
    filtered_no_user_root = 0
    filtered_small_user_subgraph = 0
    logger.info(f"\nProcessing {len(top_services)} service(s) in parallel...")
    
    # Prepare arguments for parallel processing (convert Path to string for pickling)
    process_args = [
        (service_name, service_groups.get_group(service_name).copy(), trace_col, str(graphs_dir))
        for service_name in top_services
    ]
    
    # Process services in parallel
    with ProcessPoolExecutor(max_workers=n_workers) as ex:
        futures = {ex.submit(_process_service_wrapper, args): args[0] for args in process_args}
        for fut in tqdm(as_completed(futures), total=len(futures), desc="Processing services", file=sys.stderr, dynamic_ncols=True):
            service_name = futures[fut]
            try:
                _, G, stats, has_user_root, user_subgraph_sufficient = fut.result()
                if has_user_root and user_subgraph_sufficient:
                    stats["service_name"] = service_name
                    all_stats.append(stats)
                else:
                    # Track rejection reasons separately
                    if not has_user_root:
                        filtered_no_user_root += 1
                    elif not user_subgraph_sufficient:
                        filtered_small_user_subgraph += 1
            except Exception as e:
                logger.error(f"Failed to process service {service_name}: {e!r}")
                raise
    
    # Sort statistics by number of nodes (descending)
    all_stats.sort(key=lambda x: x["num_nodes"], reverse=True)
    
    # Print statistics
    logger.info(f"{'='*80}")
    logger.info("Graph Statistics (sorted by number of nodes, descending)")
    logger.info(f"{'='*80}")
    logger.info(f"{'Service':<20} {'Traces':<10} {'Nodes':<8} {'Out-Degree':<30} {'In-Degree':<30}")
    logger.info(f"{'':-<20} {'':-<10} {'':-<8} {'':-<30} {'':-<30}")
    logger.info(f"{'':<20} {'':<10} {'':<8} {'Avg':<10} {'Min':<10} {'Max':<10} {'Avg':<10} {'Min':<10} {'Max':<10}")
    logger.info(f"{'':-<20} {'':-<10} {'':-<8} {'':-<30} {'':-<30}")
    
    for stats in all_stats:
        service_name = stats["service_name"]
        num_traces = stats["num_traces"]
        num_nodes = stats["num_nodes"]
        out_avg = stats["out_degree_avg"]
        out_min = stats["out_degree_min"]
        out_max = stats["out_degree_max"]
        in_avg = stats["in_degree_avg"]
        in_min = stats["in_degree_min"]
        in_max = stats["in_degree_max"]
        
        logger.info(
            f"{service_name:<20} {num_traces:<10,} {num_nodes:<8} "
            f"{out_avg:<10.2f} {out_min:<10} {out_max:<10} "
            f"{in_avg:<10.2f} {in_min:<10} {in_max:<10}"
        )
    
    logger.info(f"{'='*80}")
    logger.info(f"Generated graphs for {len(all_stats)} service(s) in {graphs_dir}")
    logger.info(f"Each service has its own directory with graph_all_nodes.png and graph_user.png")
    
    total_filtered = filtered_no_user_root + filtered_small_user_subgraph
    if total_filtered > 0:
        logger.info(f"\nFiltered out {total_filtered} service(s) (no graphs generated):")
        logger.info(f"  - {filtered_no_user_root} service(s) do not have 'USER' as a root node")
        logger.info(f"  - {filtered_small_user_subgraph} service(s) have a USER-reachable subgraph with fewer than 5 nodes")
    else:
        logger.info("\nAll processed services have 'USER' as a root node and USER subgraph with >= 5 nodes")

# ----------------------------
# main()
# ----------------------------

def main() -> None:
    # Configure logging to use tqdm.write() to avoid interfering with progress bars
    handler = TqdmLoggingHandler()
    handler.setFormatter(logging.Formatter("%(asctime)s - %(name)s - %(levelname)s - %(message)s"))
    logging.basicConfig(
        level=logging.INFO,
        handlers=[handler]
    )
    
    parser = argparse.ArgumentParser(
        description="Quick check script for loading CSV datasets"
    )
    parser.add_argument(
        "-n", "--num-datasets",
        type=int,
        default=10,
        help="Number of datasets to load (default: 10). Loads datasets 0 through (n-1)."
    )
    parser.add_argument(
        "-s", "--sample-fraction",
        type=float,
        default=1.0,
        help="Fraction of traces to sample from each CSV during loading (0.0 to 1.0, default: 1.0). Set to 1.0 to use all traces."
    )
    parser.add_argument(
        "--random-state",
        type=int,
        default=42,
        help="Random seed for trace sampling reproducibility (default: 42)"
    )
    parser.add_argument(
        "--max-rows",
        type=int,
        default=None,
        help="Maximum number of rows to load from each CSV file (default: None, loads all rows)"
    )
    parser.add_argument(
        "--top-services",
        type=int,
        default=100,
        help="Number of top services by trace count to process (default: 100)"
    )
    parser.add_argument(
        "--workers",
        type=int,
        default=None,
        help="Number of parallel workers for processing services (default: None, uses CPU count)"
    )
    args = parser.parse_args()
    num_datasets = args.num_datasets
    sample_fraction = args.sample_fraction
    random_state = args.random_state
    max_rows = args.max_rows
    top_services = args.top_services
    
    if num_datasets < 1:
        parser.error("Number of datasets must be at least 1")
    
    if sample_fraction <= 0.0 or sample_fraction > 1.0:
        parser.error("Sample fraction must be in (0.0, 1.0]")
    
    if max_rows is not None and max_rows < 1:
        parser.error("Max rows must be at least 1")
    
    if top_services < 1:
        parser.error("Top services must be at least 1")
    
    # Convert number of datasets to max dataset ID (0-indexed)
    max_dataset = num_datasets - 1

    # Load & concat (with sampling applied during loading)
    logger.info(f"Loading {num_datasets} dataset(s) (datasets 0 through {max_dataset})")
    if max_rows is not None:
        logger.info(f"Limiting to {max_rows:,} rows per CSV file")
    if sample_fraction < 1.0:
        logger.info(f"Sampling {sample_fraction*100:.1f}% of traces from each CSV (random_state={random_state})")
    else:
        logger.info("Using all traces (no sampling)")
    
    df = load_concat_datasets(
        max_dataset,
        max_rows=max_rows,
        sample_fraction=sample_fraction,
        random_state=random_state,
        n_workers=args.workers,
    )
    logger.info(f"Loaded {len(df):,} rows from {num_datasets} dataset(s) (after sampling)")

    # Clean data before analysis
    df = clean_data(df)

    # Analyze call graphs
    analyze_call_graphs(df, top_n=top_services, n_workers=args.workers)

if __name__ == "__main__":
    main()
