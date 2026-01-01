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
                
                # Validate that child span is within parent span (or at most equal)
                child_start = info["start_time"]
                child_end = info["end_time"]
                parent_start = parent_info["start_time"]
                parent_end = parent_info["end_time"]
                
                # Check if child span is strictly shorter than or equal to parent span
                # Child must start at or after parent and end at or before parent
                if child_start < parent_start or child_end > parent_end:
                    # Child span exceeds parent span - adjust to fit within parent
                    logger.debug(f"Adjusting child span for rpc_id {rpc_id}: child [{child_start}, {child_end}] exceeds parent [{parent_start}, {parent_end}]")
                    child_start = max(child_start, parent_start)
                    child_end = min(child_end, parent_end)
                    # Ensure child end is still after start
                    if child_end <= child_start:
                        child_end = child_start + 1  # Minimal duration
                
                # Only include if this edge exists in the graph
                if (parent_dm, child_dm) in G.edges():
                    timing_data[(parent_dm, child_dm)].append({
                        "start_time": child_start,
                        "end_time": child_end,
                        "parent_start": parent_start,
                        "parent_end": parent_end,
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

def _extract_call_sequence_from_trace(
    trace_timing: dict[tuple[str, str], list[dict]],
    parent: str,
    overlap_threshold: float = 0.1,
) -> list[set[str]]:
    """
    Extract the sequential call pattern for a parent from a single trace.
    Returns a list of sets, where each set contains children called in parallel.

    Args:
        trace_timing: Dictionary mapping (parent, child) -> list of timing dicts
        parent: Parent service name
        overlap_threshold: Fraction of overlap needed to consider calls parallel (default: 0.1)

    Returns:
        List of sets, where each set is a parallel fanout stage
        Example: [{B, C}, {D}, {E, F, G}] means B+C -> D -> E+F+G
    """
    # Get all children of this parent with their timings
    children_calls = []  # List of (child, start, end) tuples

    for (p, c), timings in trace_timing.items():
        if p == parent:
            for timing in timings:
                children_calls.append({
                    'child': c,
                    'start': timing.get('start_time', timing.get('start', 0)),
                    'end': timing.get('end_time', timing.get('end', 0)),
                })

    if not children_calls:
        return []

    # Sort all calls by start time
    children_calls.sort(key=lambda e: e['start'])

    # Group into sequential stages based on temporal ordering
    # Calls that overlap significantly are in the same stage (parallel)
    # Calls that don't overlap are in different stages (sequential)
    stages = []
    current_stage_calls = [children_calls[0]]

    for call in children_calls[1:]:
        # Check if this call overlaps with any call in the current stage
        overlaps_with_stage = False

        for stage_call in current_stage_calls:
            overlap_start = max(call['start'], stage_call['start'])
            overlap_end = min(call['end'], stage_call['end'])

            if overlap_start < overlap_end:
                # Calculate overlap ratio
                overlap_duration = overlap_end - overlap_start
                min_duration = min(call['end'] - call['start'], stage_call['end'] - stage_call['start'])
                if min_duration > 0:
                    overlap_ratio = overlap_duration / min_duration
                    if overlap_ratio >= overlap_threshold:
                        overlaps_with_stage = True
                        break

        if overlaps_with_stage:
            # Add to current stage (parallel)
            current_stage_calls.append(call)
        else:
            # Start new stage (sequential)
            if current_stage_calls:
                stages.append(set(c['child'] for c in current_stage_calls))
            current_stage_calls = [call]

    # Add final stage
    if current_stage_calls:
        stages.append(set(c['child'] for c in current_stage_calls))

    return stages

def _compute_sequence_similarity(seq1: list[set[str]], seq2: list[set[str]]) -> float:
    """
    Compute similarity between two call sequences using Jaccard similarity.

    Args:
        seq1: First sequence (list of sets)
        seq2: Second sequence (list of sets)

    Returns:
        Similarity score between 0.0 and 1.0
    """
    if not seq1 or not seq2:
        return 0.0

    # Flatten both sequences to get all children
    children1 = set()
    for stage in seq1:
        children1.update(stage)

    children2 = set()
    for stage in seq2:
        children2.update(stage)

    # Jaccard similarity of children (captures what is called, not order)
    if not children1 and not children2:
        return 1.0

    intersection = len(children1 & children2)
    union = len(children1 | children2)

    if union == 0:
        return 0.0

    jaccard = intersection / union

    # Also consider sequence length similarity
    len_similarity = 1.0 - abs(len(seq1) - len(seq2)) / max(len(seq1), len(seq2))

    # Weighted combination: 70% Jaccard (what is called), 30% length (how many stages)
    return 0.7 * jaccard + 0.3 * len_similarity

def _align_sequence_pair(
    seq1: list[set[str]],
    seq2: list[set[str]]
) -> tuple[list[set[str] | None], list[set[str] | None]]:
    """
    Align two sequences using dynamic programming to find optimal alignment.

    Args:
        seq1: First sequence
        seq2: Second sequence

    Returns:
        Tuple of (aligned_seq1, aligned_seq2) with None for gaps
    """
    n, m = len(seq1), len(seq2)

    # Score matrix: dp[i][j] = best score for aligning seq1[:i] with seq2[:j]
    dp = [[0.0] * (m + 1) for _ in range(n + 1)]

    # Traceback matrix for reconstruction
    traceback = [[None] * (m + 1) for _ in range(n + 1)]

    # Gap penalties
    gap_penalty = -0.5

    # Fill DP table
    for i in range(1, n + 1):
        dp[i][0] = i * gap_penalty
        traceback[i][0] = 'up'

    for j in range(1, m + 1):
        dp[0][j] = j * gap_penalty
        traceback[0][j] = 'left'

    for i in range(1, n + 1):
        for j in range(1, m + 1):
            # Match/mismatch score based on Jaccard similarity
            stage1 = seq1[i - 1]
            stage2 = seq2[j - 1]

            intersection = len(stage1 & stage2)
            union = len(stage1 | stage2)
            match_score = intersection / union if union > 0 else 0.0

            # Three options: match, gap in seq1, gap in seq2
            match = dp[i - 1][j - 1] + match_score
            delete = dp[i - 1][j] + gap_penalty
            insert = dp[i][j - 1] + gap_penalty

            if match >= delete and match >= insert:
                dp[i][j] = match
                traceback[i][j] = 'diag'
            elif delete >= insert:
                dp[i][j] = delete
                traceback[i][j] = 'up'
            else:
                dp[i][j] = insert
                traceback[i][j] = 'left'

    # Reconstruct alignment
    aligned1 = []
    aligned2 = []
    i, j = n, m

    while i > 0 or j > 0:
        direction = traceback[i][j]

        if direction == 'diag':
            aligned1.append(seq1[i - 1])
            aligned2.append(seq2[j - 1])
            i -= 1
            j -= 1
        elif direction == 'up':
            aligned1.append(seq1[i - 1])
            aligned2.append(None)
            i -= 1
        else:  # left
            aligned1.append(None)
            aligned2.append(seq2[j - 1])
            j -= 1

    # Reverse (we built backwards)
    aligned1.reverse()
    aligned2.reverse()

    return aligned1, aligned2

def _build_consensus_from_aligned(
    aligned_sequences: list[list[set[str] | None]]
) -> list[set[str]]:
    """
    Build a consensus sequence from multiple aligned sequences.

    Args:
        aligned_sequences: List of aligned sequences (with None for gaps)

    Returns:
        Consensus sequence (list of sets)
    """
    if not aligned_sequences:
        return []

    # Find maximum aligned length
    max_len = max(len(seq) for seq in aligned_sequences)

    consensus = []

    for pos in range(max_len):
        # Collect all children that appear at this position
        children_at_pos = defaultdict(int)
        num_non_gaps = 0

        for seq in aligned_sequences:
            if pos < len(seq) and seq[pos] is not None:
                num_non_gaps += 1
                for child in seq[pos]:
                    children_at_pos[child] += 1

        if num_non_gaps == 0:
            continue  # Skip positions that are all gaps

        # Include children that appear in at least 50% of non-gap sequences at this position
        threshold = num_non_gaps * 0.5
        consensus_stage = set()

        for child, count in children_at_pos.items():
            if count >= threshold:
                consensus_stage.add(child)

        if consensus_stage:
            consensus.append(consensus_stage)

    return consensus

def _aggregate_call_sequences(
    sequences: list[list[set[str]]],
    total_traces: int,
) -> list[dict[str, float]]:
    """
    Aggregate call sequences from multiple traces using semantic alignment.

    Algorithm:
    1. Cluster sequences by similarity
    2. For the largest cluster, perform multiple sequence alignment
    3. Build consensus pattern with probabilities

    Args:
        sequences: List of sequences, where each sequence is a list of sets (stages)
        total_traces: Total number of traces analyzed

    Returns:
        List of dicts mapping child -> probability for each sequential stage
        Example: [{'B': 0.95, 'C': 0.90}, {'D': 0.85}, {'E': 0.70, 'F': 0.65}]
    """
    if not sequences:
        return []

    if len(sequences) == 1:
        # Only one sequence - convert to probability format
        result = []
        for stage in sequences[0]:
            stage_probs = {child: 1.0 for child in stage}
            result.append(stage_probs)
        return result

    # Step 1: Find the most common sequence pattern (use as reference)
    # Use the median-length sequence that's most similar to others
    sequence_scores = []

    for i, seq in enumerate(sequences):
        total_similarity = 0.0
        for j, other_seq in enumerate(sequences):
            if i != j:
                total_similarity += _compute_sequence_similarity(seq, other_seq)
        avg_similarity = total_similarity / (len(sequences) - 1) if len(sequences) > 1 else 0.0
        sequence_scores.append((i, avg_similarity, len(seq)))

    # Sort by similarity (descending), then by length (prefer median length)
    median_len = sorted([s[2] for s in sequence_scores])[len(sequence_scores) // 2]
    sequence_scores.sort(key=lambda x: (x[1], -abs(x[2] - median_len)), reverse=True)

    reference_idx = sequence_scores[0][0]
    reference_seq = sequences[reference_idx]

    # Step 2: Align all sequences to the reference
    aligned_sequences = [reference_seq]

    for i, seq in enumerate(sequences):
        if i == reference_idx:
            continue

        aligned_ref, aligned_seq = _align_sequence_pair(reference_seq, seq)
        aligned_sequences.append(aligned_seq)

    # Step 3: Build consensus sequence
    consensus = _build_consensus_from_aligned(aligned_sequences)

    # Step 4: Calculate probabilities for each child in each consensus stage
    prob_stages = []

    for consensus_stage in consensus:
        stage_probs = {}

        for child in consensus_stage:
            # Count how many traces have this child
            count = 0
            for seq in sequences:
                # Check if child appears anywhere in this sequence
                for stage in seq:
                    if child in stage:
                        count += 1
                        break  # Count each trace only once

            probability = count / total_traces
            stage_probs[child] = probability

        if stage_probs:
            prob_stages.append(stage_probs)

    return prob_stages

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
    Parent's time is inclusive of children. Sequential children are left/right,
    parallel children are vertically aligned. This is done recursively for all layers.
    
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
    # First, normalize all timings relative to root (USER) start
    # Find the earliest parent_start across all timing data (this is the root start)
    root_start = None
    for timings in timing_data.values():
        for timing in timings:
            if root_start is None or timing["parent_start"] < root_start:
                root_start = timing["parent_start"]
    
    if root_start is None:
        plt.close()
        return
    
    # Normalize all timings relative to root start
    edge_timelines = {}
    for (parent, child), timings in timing_data.items():
        if not timings or (parent, child) not in G.edges():
            continue
        
        normalized_timings = []
        for timing in timings:
            relative_start = timing["start_time"] - root_start
            relative_end = timing["end_time"] - root_start
            normalized_timings.append({
                "start": relative_start,
                "end": relative_end,
                "start_time": timing["start_time"],  # Keep original for classification
                "end_time": timing["end_time"],  # Keep original for classification
                "parent_start": timing["parent_start"] - root_start,  # Normalized
                "parent_end": timing["parent_end"] - root_start,  # Normalized
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
    
    # Colors for different patterns
    colors = {'sequential': '#2E86AB', 'parallel': '#A23B72', 'mixed': '#F18F01'}
    
    # Build node timing information recursively
    # For each node, calculate its inclusive time span (from earliest child to latest child)
    node_timings = {}  # node -> {"start": float, "end": float, "has_children": bool}
    calculating = set()  # Track nodes currently being calculated to detect cycles
    max_recursion_depth = 1000  # Safety limit
    
    def calculate_node_timing(node: str, depth: int = 0) -> dict:
        """Recursively calculate inclusive timing for a node.
        
        Args:
            node: Node to calculate timing for
            depth: Current recursion depth (for cycle detection)
        
        Returns:
            Dictionary with start, end, and has_children keys
        """
        # Check if already calculated
        if node in node_timings:
            return node_timings[node]
        
        # Check for cycles or excessive recursion
        if node in calculating:
            # Cycle detected - return placeholder to break recursion
            logger.warning(f"Cycle detected in graph at node '{node}', using placeholder timing")
            node_timings[node] = {"start": 0.0, "end": 0.1 * max_time, "has_children": False}
            return node_timings[node]
        
        if depth > max_recursion_depth:
            # Excessive recursion - likely a very deep graph or cycle
            logger.warning(f"Maximum recursion depth exceeded for node '{node}', using placeholder timing")
            node_timings[node] = {"start": 0.0, "end": 0.1 * max_time, "has_children": False}
            return node_timings[node]
        
        # Mark as currently calculating
        calculating.add(node)
        
        try:
            children = list(G.successors(node))
            
            if not children:
                # Leaf node - use its own timing if available
                # Find timing from any edge where this node is a child
                min_start = None
                max_end = None
                for (parent, child), timings in edge_timelines.items():
                    if child == node and timings:
                        for timing in timings:
                            if min_start is None or timing["start"] < min_start:
                                min_start = timing["start"]
                            if max_end is None or timing["end"] > max_end:
                                max_end = timing["end"]
                
                if min_start is not None and max_end is not None:
                    node_timings[node] = {"start": min_start, "end": max_end, "has_children": False}
                else:
                    # No timing data - use placeholder
                    node_timings[node] = {"start": 0.0, "end": 0.1 * max_time, "has_children": False}
                return node_timings[node]
            
            # Parent node - calculate from children
            child_starts = []
            child_ends = []
            
            for child in children:
                child_timing = calculate_node_timing(child, depth + 1)
                child_starts.append(child_timing["start"])
                child_ends.append(child_timing["end"])
            
            if child_starts and child_ends:
                node_start = min(child_starts)
                node_end = max(child_ends)
                # Ensure parent end is strictly greater than all child ends
                # Add a small buffer to ensure children are narrower than parent
                if node_end <= max(child_ends):
                    node_end = max(child_ends) + 0.01 * max_time
            else:
                # No child timing data - use placeholder
                node_start = 0.0
                node_end = 0.1 * max_time
            
            node_timings[node] = {"start": node_start, "end": node_end, "has_children": True}
            return node_timings[node]
        
        finally:
            # Always remove from calculating set when done
            calculating.discard(node)
    
    # Calculate timings for all nodes (starting from root)
    if "USER" in G.nodes():
        calculate_node_timing("USER")
        # Also calculate for all other nodes
        for node in G.nodes():
            if node not in node_timings:
                calculate_node_timing(node)
    else:
        # No USER node - calculate for all nodes
        for node in G.nodes():
            if node not in node_timings:
                calculate_node_timing(node)
    
    # Determine if children are sequential or parallel
    def are_children_sequential(parent: str, children: list[str], overlap_threshold: float = 0.1) -> dict[str, bool]:
        """Determine which children are sequential vs parallel."""
        if len(children) < 2:
            return {child: True for child in children}
        
        result = {}
        child_ranges = {}
        
        # Get timing ranges for each child
        for child in children:
            if child in node_timings:
                child_ranges[child] = (node_timings[child]["start"], node_timings[child]["end"])
            else:
                child_ranges[child] = (0.0, 0.1 * max_time)
        
        # Sort children by start time
        sorted_children = sorted(children, key=lambda c: child_ranges[c][0])
        
        # Check each child against previous ones
        for i, child in enumerate(sorted_children):
            is_sequential = True
            child_start, child_end = child_ranges[child]
            
            # Check if this child overlaps significantly with any previous child
            for prev_child in sorted_children[:i]:
                prev_start, prev_end = child_ranges[prev_child]
                
                # Check overlap
                overlap_start = max(prev_start, child_start)
                overlap_end = min(prev_end, child_end)
                
                if overlap_start < overlap_end:
                    overlap_duration = overlap_end - overlap_start
                    min_duration = min(prev_end - prev_start, child_end - child_start)
                    if min_duration > 0:
                        overlap_ratio = overlap_duration / min_duration
                        if overlap_ratio >= overlap_threshold:
                            is_sequential = False
                            break
            
            result[child] = is_sequential
        
        return result
    
    # Position nodes on timeline with proper spacing to prevent overlaps
    node_x_positions = {}  # node -> x position (start time)
    node_y_timeline_positions = {}  # node -> y position on timeline
    node_visual_timings = {}  # node -> visual timing for display
    layout_in_progress = set()  # Track nodes currently being laid out (cycle detection)

    def layout_subtree(node: str, parent_x_start: float, parent_x_end: float,
                       base_y: float, depth: int = 0) -> float:
        """
        Recursively layout a subtree, returning the minimum y position used.

        Args:
            node: Current node to layout
            parent_x_start: Parent's start time (or 0 for root)
            parent_x_end: Parent's end time (or max_time for root)
            base_y: Y position for this node
            depth: Recursion depth (for cycle detection)

        Returns:
            Minimum y position used by this subtree
        """
        # Check if already positioned (avoid re-processing)
        if node in node_y_timeline_positions:
            return node_y_timeline_positions[node]

        # Check for cycles
        if node in layout_in_progress:
            logger.warning(f"Cycle detected in graph at node '{node}', skipping to prevent infinite recursion")
            # Position node with placeholder to break cycle
            node_x_positions[node] = parent_x_start
            node_y_timeline_positions[node] = base_y
            node_visual_timings[node] = {
                "start": parent_x_start,
                "end": min(parent_x_start + 0.01 * max_time, parent_x_end),
                "has_children": False
            }
            return base_y

        if depth > 100:
            logger.warning(f"Maximum recursion depth exceeded for node '{node}'")
            return base_y

        # Mark as being processed
        layout_in_progress.add(node)

        try:
            # Get timing for this node
            timing = node_timings.get(node, {"start": 0.0, "end": 0.1 * max_time})

            # Ensure node timing fits within parent bounds (with small inset)
            inset = 0.01 * max_time
            node_start = max(timing["start"], parent_x_start + inset)
            node_end = min(timing["end"], parent_x_end - inset)

            # Ensure end > start
            if node_end <= node_start:
                node_end = node_start + 0.01 * max_time

            # Position this node
            node_x_positions[node] = node_start
            node_y_timeline_positions[node] = base_y
            node_visual_timings[node] = {
                "start": node_start,
                "end": node_end,
                "has_children": timing.get("has_children", False)
            }

            # Get children
            children = list(G.successors(node))
            if not children:
                return base_y  # Leaf node, return current y

            # Determine which children are sequential vs parallel
            seq_parallel = are_children_sequential(node, children)

            # Sort children by their original start time
            children_sorted = sorted(children, key=lambda c: node_timings.get(c, {}).get("start", 0.0))

            # Group into sequential segments and parallel groups
            groups = []  # List of (is_parallel, [children])
            current_parallel = []

            for child in children_sorted:
                if not seq_parallel.get(child, True):  # Parallel
                    current_parallel.append(child)
                else:  # Sequential
                    if current_parallel:
                        groups.append((True, current_parallel))
                        current_parallel = []
                    groups.append((False, [child]))

            if current_parallel:
                groups.append((True, current_parallel))

            # Layout children with proper time budget allocation
            bar_height = 0.1
            gap = 0.05
            inter_child_gap = 0.005 * max_time
            child_base_y = base_y - (bar_height + gap)

            # Calculate available time for children
            available_time = node_end - node_start - 2 * inset

            if available_time <= 0:
                available_time = 0.01 * max_time

            # First pass: Calculate how much time each group needs
            group_time_needs = []
            total_time_needed = 0

            for is_parallel, group in groups:
                if is_parallel:
                    # Parallel group: needs time for the longest child
                    group_timings = [node_timings.get(c, {"start": 0.0, "end": 0.1 * max_time}) for c in group]
                    max_duration = max(t["end"] - t["start"] for t in group_timings)
                    group_time_needs.append(max_duration)
                    total_time_needed += max_duration
                else:
                    # Sequential group: needs sum of all children's durations
                    group_duration = 0
                    for child in group:
                        child_timing = node_timings.get(child, {"start": 0.0, "end": 0.1 * max_time})
                        child_duration = child_timing["end"] - child_timing["start"]
                        group_duration += child_duration
                    group_time_needs.append(group_duration)
                    total_time_needed += group_duration

            # Add gaps between groups
            if len(groups) > 1:
                total_time_needed += inter_child_gap * (len(groups) - 1)

            # Calculate scaling factor if children don't fit
            if total_time_needed > available_time:
                # Need to compress children to fit within parent
                scale_factor = (available_time - inter_child_gap * max(0, len(groups) - 1)) / (total_time_needed - inter_child_gap * max(0, len(groups) - 1))
                scale_factor = max(0.05, scale_factor)  # Minimum 5% of original size
            else:
                scale_factor = 1.0

            # Second pass: Layout children with allocated time budgets
            current_x = node_start + inset
            min_y_used = child_base_y

            for idx, (is_parallel, group) in enumerate(groups):
                allocated_time = group_time_needs[idx] * scale_factor

                # Ensure minimum allocation
                allocated_time = max(allocated_time, 0.01 * max_time)

                # Ensure we don't exceed parent bounds
                group_end = min(current_x + allocated_time, node_end - inset)

                if group_end <= current_x:
                    group_end = current_x + 0.01 * max_time

                if is_parallel:
                    # Parallel children: stack vertically, same x range
                    current_y = child_base_y
                    for child in group:
                        child_min_y = layout_subtree(child, current_x, group_end, current_y, depth + 1)
                        # Move down for next parallel sibling
                        current_y = child_min_y - (bar_height + gap)
                        min_y_used = min(min_y_used, child_min_y)

                    # Move current_x past the parallel section
                    current_x = group_end + inter_child_gap

                else:
                    # Sequential children: distribute allocated time among them
                    # Calculate individual time budgets based on original proportions
                    group_timings = []
                    total_group_duration = 0
                    for child in group:
                        child_timing = node_timings.get(child, {"start": 0.0, "end": 0.1 * max_time})
                        child_duration = child_timing["end"] - child_timing["start"]
                        group_timings.append(child_duration)
                        total_group_duration += child_duration

                    # Allocate time proportionally
                    child_x = current_x
                    for i, child in enumerate(group):
                        if total_group_duration > 0:
                            # Proportional allocation
                            child_allocated = allocated_time * (group_timings[i] / total_group_duration)
                        else:
                            # Equal allocation if no timing info
                            child_allocated = allocated_time / len(group)

                        # Ensure minimum size
                        child_allocated = max(child_allocated, 0.01 * max_time)

                        child_end = min(child_x + child_allocated, node_end - inset)

                        if child_end <= child_x:
                            child_end = child_x + 0.01 * max_time

                        child_min_y = layout_subtree(child, child_x, child_end, child_base_y, depth + 1)
                        min_y_used = min(min_y_used, child_min_y)

                        # Move to next sequential child (with small gap)
                        child_x = child_end + inter_child_gap

                    # Update current_x to after all sequential children
                    current_x = child_x

            return min_y_used

        finally:
            # Always remove from in-progress set when done
            layout_in_progress.discard(node)

    # Start layout from root
    if "USER" in G.nodes():
        root_y = max_level if max_level > 0 else 0.0
        layout_subtree("USER", 0.0, max_time, root_y)
    else:
        # Multiple roots
        roots = [n for n in G.nodes() if G.in_degree(n) == 0]
        root_y = max_level if max_level > 0 else 0.0
        for root in roots:
            layout_subtree(root, 0.0, max_time, root_y)

    # Ensure all nodes are positioned (handle disconnected components)
    for node in G.nodes():
        if node not in node_y_timeline_positions:
            timing = node_timings.get(node, {"start": 0.0, "end": 0.1 * max_time})
            node_x_positions[node] = timing["start"]
            node_y_timeline_positions[node] = 0.0
            node_visual_timings[node] = timing.copy()
    
    # Draw timeline bars
    # Sort nodes by y position (lowest first = bottom first)
    nodes_by_y = sorted(G.nodes(), key=lambda n: node_y_timeline_positions.get(n, 0))
    
    bar_height = 0.1
    gap = 0.05
    
    # Track label positions to prevent overlaps
    label_positions = []  # List of (x_start, x_end, y, text) tuples
    
    def check_label_overlap(x_start: float, x_end: float, y: float, text: str, fontsize: int = 7) -> tuple[float, float]:
        """Check if label would overlap with existing labels, adjust if needed.
        
        Returns:
            (label_x, label_y) position for the label
        """
        # Estimate text width in data coordinates
        char_width = 0.008 * max_time
        text_width = char_width * len(text)
        text_height = bar_height
        
        # Try to place label in the middle of the bar
        bar_center_x = x_start + (x_end - x_start) / 2
        label_x = bar_center_x - text_width / 2
        
        # Check for overlaps with existing labels at similar y positions
        for existing_x_start, existing_x_end, existing_y, _ in label_positions:
            # Check if y positions are close (within bar height)
            if abs(y - existing_y) < text_height * 1.2:
                # Check if x positions overlap
                if not (label_x + text_width < existing_x_start or label_x > existing_x_end):
                    # Overlap detected - try to shift left or right
                    if existing_x_end < bar_center_x:
                        # Existing label is to the left, shift right
                        label_x = existing_x_end + 0.01 * max_time
                    else:
                        # Existing label is to the right, shift left
                        label_x = existing_x_start - text_width - 0.01 * max_time
        
        # Clamp to bar bounds (with small padding)
        padding = 0.01 * max_time
        if label_x < x_start + padding:
            label_x = x_start + padding
        if label_x + text_width > x_end - padding:
            label_x = x_end - text_width - padding
            # If still doesn't fit, just center it
            if label_x < x_start + padding:
                label_x = bar_center_x - text_width / 2
        
        # Record this label position
        label_positions.append((label_x, label_x + text_width, y, text))
        
        return label_x, y
    
    # Draw all nodes
    for node in nodes_by_y:
        if node not in node_y_timeline_positions:
            continue
        
        visual_timing = node_visual_timings.get(node, node_timings.get(node, {"start": 0.0, "end": 0.1 * max_time}))
        x_pos = node_x_positions.get(node, visual_timing["start"])
        y_pos = node_y_timeline_positions[node]
        
        # Draw node bar using visual timing
        min_width = 0.01 * max_time
        width = max(min_width, visual_timing["end"] - visual_timing["start"])
        
        # Determine color based on children pattern
        children = list(G.successors(node))
        if children:
            child_timings_dict = {}
            for child in children:
                edge = (node, child)
                if edge in edge_timelines:
                    child_timings_dict[child] = edge_timelines[edge]
            
            if child_timings_dict:
                pattern_type, _ = _classify_call_pattern(child_timings_dict)
                color = colors.get(pattern_type, 'gray')
            else:
                color = 'gray'
        else:
            # Leaf node - use light gray
            color = 'lightgray'
        
        # Draw bar
        ax_right.barh(y_pos, width, left=x_pos, height=bar_height, 
                     color=color, alpha=0.7, edgecolor='black', linewidth=1.0, zorder=2)
        
        # Draw node label INSIDE the bar (centered, with overlap checking)
        label_x, label_y = check_label_overlap(x_pos, x_pos + width, y_pos, node, fontsize=7)
        
        # Use white text with semi-transparent background for better visibility
        ax_right.text(label_x, label_y, node, 
                     fontsize=7, ha='left', va='center', fontweight='bold',
                     color='white', zorder=3,
                     bbox=dict(boxstyle='round,pad=0.1', facecolor='black', alpha=0.4, edgecolor='white', linewidth=0.5))
    
    # Draw arrows from parents to children (straight arrows pointing down)
    for node in G.nodes():
        if node not in node_y_timeline_positions or node not in node_x_positions:
            continue
        
        node_visual_timing = node_visual_timings.get(node, node_timings.get(node, {"start": 0.0, "end": 0.1 * max_time}))
        node_x = node_x_positions[node]
        node_y = node_y_timeline_positions[node]
        
        # Draw arrows to all children
        for child in G.successors(node):
            if child not in node_y_timeline_positions or child not in node_x_positions:
                continue
            
            child_visual_timing = node_visual_timings.get(child, node_timings.get(child, {"start": 0.0, "end": 0.1 * max_time}))
            child_x = node_x_positions[child]
            child_y = node_y_timeline_positions[child]
            
            # Arrow starts from left side of parent span
            parent_left_x = node_x
            # Arrow points to center of child span
            child_center_x = child_x + (child_visual_timing["end"] - child_visual_timing["start"]) / 2
            
            # Draw straight arrow from parent bottom-left to child top-center
            ax_right.annotate('', xy=(child_center_x, child_y + bar_height / 2), 
                            xytext=(parent_left_x, node_y - bar_height / 2),
                            arrowprops=dict(arrowstyle='->', color='black', lw=1.5, alpha=0.5, mutation_scale=15),
                            zorder=1)
    
    # Set axis properties
    # Calculate y limits based on node positions
    if node_y_timeline_positions:
        all_y_positions = list(node_y_timeline_positions.values())
        min_y = min(all_y_positions) - 0.5
        max_y = max(all_y_positions) + 0.5
    else:
        min_y = -0.5
        max_y = max_level + 0.5
    
    ax_right.set_xlim(-0.05 * max_time, max_time * 1.1)
    ax_right.set_ylim(min_y, max_y)
    ax_right.set_xlabel('Time (normalized to root start)', fontsize=10)
    ax_right.set_ylabel('Hierarchy Level (from USER)', fontsize=10)
    ax_right.set_title('Call Pattern Timeline (Parent time is inclusive of children)', 
                     fontsize=10, fontweight='bold')
    ax_right.grid(True, alpha=0.3, axis='both')
    
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

def _draw_call_sequence_graph(
    parent: str,
    call_sequence: list[dict[str, float]],
    output_path: Path,
) -> None:
    """
    Draw a call sequence diagram showing sequential stages with parallel fanout and probabilities.

    Args:
        parent: Parent service name
        call_sequence: List of dicts mapping child -> probability for each stage
        output_path: Path to save the image
    """
    if not call_sequence:
        return

    num_stages = len(call_sequence)

    # Calculate figure size based on complexity
    max_fanout = max(len(stage) for stage in call_sequence)
    fig_width = max(12, num_stages * 4)
    fig_height = max(8, max_fanout * 1.5)

    fig, ax = plt.subplots(figsize=(fig_width, fig_height))

    # Layout parameters
    stage_spacing = 1.0 / (num_stages + 1)  # Horizontal spacing between stages
    node_height = 0.8 / max_fanout if max_fanout > 0 else 0.1  # Vertical spacing

    # Track node positions for drawing edges
    node_positions = {}

    # Draw parent node
    parent_x = 0.05
    parent_y = 0.5
    node_positions[parent] = (parent_x, parent_y)

    ax.add_patch(plt.Rectangle((parent_x - 0.03, parent_y - 0.03), 0.06, 0.06,
                                facecolor='lightgreen', edgecolor='black', linewidth=2))
    ax.text(parent_x, parent_y, parent, ha='center', va='center',
            fontsize=10, fontweight='bold')

    # Draw each stage
    for stage_idx, stage_probs in enumerate(call_sequence):
        stage_x = 0.15 + (stage_idx + 1) * stage_spacing

        # Sort children by probability (descending) for consistent layout
        sorted_children = sorted(stage_probs.items(), key=lambda x: x[1], reverse=True)
        num_children = len(sorted_children)

        # Calculate vertical positions (centered)
        if num_children == 1:
            y_positions = [0.5]
        else:
            y_start = 0.5 - (num_children - 1) * node_height / 2
            y_positions = [y_start + i * node_height for i in range(num_children)]

        # Draw stage label
        ax.text(stage_x, 0.95, f'Stage {stage_idx + 1}', ha='center', va='top',
                fontsize=9, fontweight='bold', style='italic', color='gray')

        # Draw children in this stage
        for child_idx, ((child, prob), y_pos) in enumerate(zip(sorted_children, y_positions)):
            node_positions[f"{child}_stage_{stage_idx}"] = (stage_x, y_pos)

            # Color based on probability (green = high, yellow = medium, red = low)
            if prob >= 0.8:
                color = '#90EE90'  # Light green
            elif prob >= 0.5:
                color = '#FFD700'  # Gold
            else:
                color = '#FFB6C1'  # Light pink

            # Draw node rectangle
            ax.add_patch(plt.Rectangle((stage_x - 0.035, y_pos - 0.025), 0.07, 0.05,
                                       facecolor=color, edgecolor='black', linewidth=1.5,
                                       alpha=0.8))

            # Draw child name and probability
            ax.text(stage_x, y_pos + 0.01, child, ha='center', va='center',
                    fontsize=8, fontweight='bold')
            ax.text(stage_x, y_pos - 0.015, f'p={prob:.2f}', ha='center', va='center',
                    fontsize=7, style='italic', color='darkblue')

    # Draw arrows from parent to first stage
    if call_sequence:
        first_stage = call_sequence[0]
        stage_x = 0.15 + stage_spacing
        sorted_children = sorted(first_stage.items(), key=lambda x: x[1], reverse=True)
        num_children = len(sorted_children)

        if num_children == 1:
            y_positions = [0.5]
        else:
            y_start = 0.5 - (num_children - 1) * node_height / 2
            y_positions = [y_start + i * node_height for i in range(num_children)]

        for (child, prob), y_pos in zip(sorted_children, y_positions):
            ax.annotate('', xy=(stage_x - 0.035, y_pos), xytext=(parent_x + 0.03, parent_y),
                       arrowprops=dict(arrowstyle='->', color='black', lw=1.5, alpha=0.6))

    # Draw arrows between sequential stages
    for stage_idx in range(len(call_sequence) - 1):
        current_stage = call_sequence[stage_idx]
        next_stage = call_sequence[stage_idx + 1]

        current_x = 0.15 + (stage_idx + 1) * stage_spacing
        next_x = 0.15 + (stage_idx + 2) * stage_spacing

        # Get positions for current stage
        sorted_current = sorted(current_stage.items(), key=lambda x: x[1], reverse=True)
        num_current = len(sorted_current)
        if num_current == 1:
            current_y_positions = [0.5]
        else:
            y_start = 0.5 - (num_current - 1) * node_height / 2
            current_y_positions = [y_start + i * node_height for i in range(num_current)]

        # Get positions for next stage
        sorted_next = sorted(next_stage.items(), key=lambda x: x[1], reverse=True)
        num_next = len(sorted_next)
        if num_next == 1:
            next_y_positions = [0.5]
        else:
            y_start = 0.5 - (num_next - 1) * node_height / 2
            next_y_positions = [y_start + i * node_height for i in range(num_next)]

        # Draw arrows from each child in current stage to each child in next stage
        for current_y in current_y_positions:
            for next_y in next_y_positions:
                ax.annotate('', xy=(next_x - 0.035, next_y), xytext=(current_x + 0.035, current_y),
                           arrowprops=dict(arrowstyle='->', color='gray', lw=1.0, alpha=0.3))

    # Set axis properties
    ax.set_xlim(-0.05, 1.05)
    ax.set_ylim(-0.05, 1.05)
    ax.axis('off')

    # Add title
    title = f"Call Sequence Pattern for Parent '{parent}'\n(Sequential stages with parallel fanout and probabilities)"
    plt.title(title, fontsize=12, fontweight='bold', pad=20)

    # Add legend
    legend_elements = [
        mpatches.Patch(facecolor='#90EE90', label='High Probability (≥0.8)', alpha=0.8),
        mpatches.Patch(facecolor='#FFD700', label='Medium Probability (0.5-0.8)', alpha=0.8),
        mpatches.Patch(facecolor='#FFB6C1', label='Low Probability (<0.5)', alpha=0.8),
    ]
    ax.legend(handles=legend_elements, loc='lower right', fontsize=8)

    plt.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()

def _build_hierarchical_call_tree(
    parent_sequences: dict[str, list[dict[str, float]]],
    root: str = "USER",
    max_depth: int = 10,
) -> dict:
    """
    Build a hierarchical call tree by recursively chaining sequences together.
    Preserves sequential ordering and parallel grouping from call sequences.

    Args:
        parent_sequences: Dict mapping parent -> call_sequence
        root: Root node to start from (default: "USER")
        max_depth: Maximum recursion depth to prevent infinite loops

    Returns:
        Nested dict representing the call tree with stages:
        {
            "name": "USER",
            "prob": 1.0,
            "stages": [
                {
                    "stage_num": 1,
                    "children": [
                        {"name": "A", "prob": 0.95, "stages": [...]},
                        {"name": "B", "prob": 0.92, "stages": [...]}
                    ]
                },
                {
                    "stage_num": 2,
                    "children": [
                        {"name": "C", "prob": 0.90, "stages": [...]}
                    ]
                }
            ]
        }
    """
    visited = set()

    def _build_tree_recursive(node: str, prob: float, depth: int) -> dict:
        """Recursively build tree for a given node, preserving stages."""
        if depth >= max_depth:
            logger.debug(f"Max depth ({max_depth}) reached for node '{node}' at depth {depth}")
            return {"name": node, "prob": prob, "stages": []}
        if node in visited:
            logger.debug(f"Node '{node}' already being processed (cycle detected)")
            return {"name": node, "prob": prob, "stages": []}

        visited.add(node)
        tree_node = {"name": node, "prob": prob, "stages": []}

        # If this node has its own call sequence, expand it
        if node in parent_sequences:
            logger.debug(f"Expanding node '{node}' at depth {depth} (has {len(parent_sequences[node])} stages)")
            call_sequence = parent_sequences[node]

            # Process each stage in the sequence (preserves ordering)
            for stage_idx, stage_probs in enumerate(call_sequence):
                stage_children = []

                # For each child in this stage, recursively build subtree
                for child, child_prob in stage_probs.items():
                    subtree = _build_tree_recursive(child, child_prob, depth + 1)
                    stage_children.append(subtree)

                # Add this stage with all its parallel children
                tree_node["stages"].append({
                    "stage_num": stage_idx + 1,
                    "children": stage_children
                })
        else:
            logger.debug(f"Node '{node}' has no call sequence (leaf node or not in parent_sequences)")

        visited.discard(node)  # Allow node to appear in different branches
        return tree_node

    # Start building from root
    if root not in parent_sequences:
        logger.warning(f"Root node '{root}' not found in parent_sequences")
        return {"name": root, "prob": 1.0, "stages": []}

    return _build_tree_recursive(root, 1.0, 0)


def _compute_tree_layout(tree: dict, x_spacing: float = 1.0, y_spacing: float = 1.0) -> dict[str, tuple[float, float]]:
    """
    Compute (x, y) positions for all nodes in the tree using a hierarchical layout.

    Args:
        tree: Tree structure from _build_hierarchical_call_tree
        x_spacing: Horizontal spacing between nodes
        y_spacing: Vertical spacing between levels

    Returns:
        Dict mapping node_id -> (x, y) position
    """
    positions = {}
    node_counter = [0]  # Use list to make it mutable in nested function

    def _layout_recursive(node: dict, depth: int, parent_x: float | None = None) -> tuple[float, float]:
        """
        Recursively compute positions using a top-down approach.
        Returns (x, y) position of this node.
        """
        node_id = f"{node['name']}_{node_counter[0]}"
        node_counter[0] += 1

        y = -depth * y_spacing  # Top to bottom

        if not node["children"]:
            # Leaf node - place at next available x position
            x = len([p for p in positions.values() if p[1] == y]) * x_spacing
            positions[node_id] = (x, y)
            return x, y

        # Internal node - recursively layout children first
        child_positions = []
        for child in node["children"]:
            child_x, child_y = _layout_recursive(child, depth + 1, None)
            child_positions.append(child_x)

        # Place this node at the midpoint of its children
        if child_positions:
            x = (min(child_positions) + max(child_positions)) / 2
        else:
            x = 0

        positions[node_id] = (x, y)
        return x, y

    _layout_recursive(tree, 0)
    return positions


def _draw_unified_call_sequence_graph(
    parent_sequences: dict[str, list[dict[str, float]]],
    output_path: Path,
    service_name: str,
) -> None:
    """
    Draw a unified hierarchical call sequence tree starting from USER root.

    Visualization:
    - X-axis: Timeline (stages progress left to right)
    - Y-axis: Call depth (USER at top, children below, grandchildren further below)

    Args:
        parent_sequences: Dict mapping parent -> call_sequence
        output_path: Path to save the image
        service_name: Name of the service (for title)
    """
    if not parent_sequences:
        return

    # Build hierarchical tree starting from USER
    root = "USER"
    if root not in parent_sequences:
        logger.warning(f"Root node '{root}' not found in parent_sequences, cannot create unified tree")
        return

    tree = _build_hierarchical_call_tree(parent_sequences, root=root, max_depth=20)

    # Debug: Print tree structure
    def _debug_print_tree(node: dict, indent: int = 0):
        """Debug helper to print tree structure."""
        prefix = "  " * indent
        logger.info(f"{prefix}{node['name']} (prob={node['prob']:.2f}, stages={len(node['stages'])})")
        for stage in node["stages"]:
            logger.info(f"{prefix}  Stage {stage['stage_num']} ({len(stage['children'])} children)")
            for child in stage["children"]:
                _debug_print_tree(child, indent + 2)

    logger.info(f"\n{'='*60}\nDEBUG: Tree structure for {service_name}:")
    _debug_print_tree(tree)
    logger.info(f"{'='*60}\n")

    # Flatten tree to get all nodes and edges with positions
    nodes_list = []
    edges_list = []
    node_counter = [0]

    def _traverse_tree(node: dict, parent_id: str | None = None, parent_x: float = 0, tree_depth: int = 0):
        """
        Traverse tree and collect nodes/edges.

        Args:
            node: Current node in tree
            parent_id: ID of parent node
            parent_x: X position where parent appears (timeline position)
            tree_depth: Depth in call hierarchy (USER=0, children=1, etc.)
        """
        node_id = f"{node['name']}_{node_counter[0]}"
        node_counter[0] += 1

        # This node appears at parent's X position
        node_x = parent_x
        node_y = tree_depth

        nodes_list.append({
            "id": node_id,
            "name": node["name"],
            "prob": node["prob"],
            "x": node_x,
            "y": node_y,
            "tree_depth": tree_depth
        })

        if parent_id is not None:
            edges_list.append((parent_id, node_id))

        # Process stages sequentially (each stage advances timeline)
        current_x = parent_x
        for stage in node["stages"]:
            stage_num = stage["stage_num"]
            # Advance timeline for this stage
            stage_x = parent_x + stage_num

            # Process children in this stage (all at same X, different Y)
            for child in stage["children"]:
                _traverse_tree(child, node_id, stage_x, tree_depth + 1)

    _traverse_tree(tree)

    logger.info(f"DEBUG: Found {len(nodes_list)} total nodes in tree, {len(edges_list)} edges")
    logger.info(f"DEBUG: Nodes by depth: {dict(sorted([(d, len([n for n in nodes_list if n['tree_depth'] == d])) for d in set(n['tree_depth'] for n in nodes_list)]))}")

    if not nodes_list:
        logger.warning("No nodes found in hierarchical tree")
        return

    # Create position dictionary
    pos = {node["id"]: (node["x"], -node["y"]) for node in nodes_list}  # Negative Y so USER is at top

    # Compute axis ranges
    max_x = max(node["x"] for node in nodes_list)
    max_y = max(node["y"] for node in nodes_list)

    # Create figure
    x_spacing = 200  # Pixels per stage
    y_spacing = 150  # Pixels per depth level

    fig_width = max(16, (max_x + 2) * x_spacing / 100)
    fig_height = max(10, (max_y + 2) * y_spacing / 100)

    fig, ax = plt.subplots(figsize=(fig_width, fig_height))

    # Scale positions for better visualization
    scaled_pos = {
        node_id: (x * x_spacing, y * y_spacing)
        for node_id, (x, y) in pos.items()
    }

    # Draw edges
    for parent_id, child_id in edges_list:
        x1, y1 = scaled_pos[parent_id]
        x2, y2 = scaled_pos[child_id]

        ax.annotate('', xy=(x2, y2 + 40), xytext=(x1, y1 - 40),
                   arrowprops=dict(arrowstyle='->', color='gray',
                                 lw=1.5, alpha=0.6))

    # Draw grid lines for timeline stages
    for x_stage in range(int(max_x) + 2):
        x_pos = x_stage * x_spacing
        ax.axvline(x=x_pos, color='lightgray', linestyle='--', linewidth=0.5, alpha=0.3, zorder=0)
        # Label the stage at top
        ax.text(x_pos, max_y * y_spacing + 60, f'S{x_stage}' if x_stage > 0 else 'Start',
               ha='center', va='bottom', fontsize=8, color='gray', fontweight='bold')

    # Draw horizontal lines for depth levels
    for y_depth in range(int(max_y) + 1):
        y_pos = -y_depth * y_spacing
        ax.axhline(y=y_pos, color='lightgray', linestyle='--', linewidth=0.5, alpha=0.3, zorder=0)
        # Label the depth on the left
        ax.text(-x_spacing * 0.3, y_pos, f'Depth {y_depth}',
               ha='right', va='center', fontsize=8, color='gray', style='italic')

    # Draw nodes
    for node in nodes_list:
        node_id = node["id"]
        x, y = scaled_pos[node_id]
        prob = node["prob"]
        name = node["name"]
        tree_depth = node["tree_depth"]

        # Color based on probability
        if prob >= 0.8:
            color = '#90EE90'
        elif prob >= 0.5:
            color = '#FFD700'
        else:
            color = '#FFB6C1'

        # Special color for root
        if tree_depth == 0:
            color = '#87CEEB'  # Sky blue for root

        # Draw node circle
        circle = plt.Circle((x, y), radius=40, facecolor=color, edgecolor='black',
                           linewidth=2.0, alpha=0.9, zorder=2)
        ax.add_patch(circle)

        # Draw node label
        ax.text(x, y, name, ha='center', va='center',
               fontsize=9, fontweight='bold', zorder=3)

        # Draw probability below node
        if tree_depth > 0:  # Don't show prob for root
            ax.text(x, y - 55, f'{prob:.2f}', ha='center', va='top',
                   fontsize=7, color='darkblue', zorder=3)

    # Set axis properties
    ax.set_aspect('equal')
    ax.axis('off')

    # Set axis limits with margins
    x_margin = x_spacing * 0.5
    y_margin = y_spacing * 0.5
    ax.set_xlim(-x_spacing * 0.5, (max_x + 1) * x_spacing + x_margin)
    ax.set_ylim(-(max_y + 1) * y_spacing - y_margin, y_spacing)

    # Add title
    title = f"Unified Call Sequence Timeline for Service '{service_name}'\n" \
            f"(X-axis: Timeline/Stages | Y-axis: Call Depth)"
    plt.title(title, fontsize=14, fontweight='bold', pad=20)

    # Add legend
    legend_elements = [
        mpatches.Patch(facecolor='#87CEEB', label='Root (USER)', alpha=0.9),
        mpatches.Patch(facecolor='#90EE90', label='High Prob (≥0.8)', alpha=0.9),
        mpatches.Patch(facecolor='#FFD700', label='Med Prob (0.5-0.8)', alpha=0.9),
        mpatches.Patch(facecolor='#FFB6C1', label='Low Prob (<0.5)', alpha=0.9),
    ]
    ax.legend(handles=legend_elements, loc='upper right', fontsize=9)

    # Add axis labels
    ax.text(0.5, -0.02, '→ Timeline (Sequential Stages) →',
            ha='center', va='top', fontsize=10, style='italic', color='gray',
            fontweight='bold', transform=ax.transAxes)

    ax.text(-0.02, 0.5, '← Call Depth (Hierarchy) ←',
            ha='right', va='center', fontsize=10, style='italic', color='gray',
            fontweight='bold', rotation=90, transform=ax.transAxes)

    plt.tight_layout()
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()

def _process_service(
    service_name: str,
    service_df: pd.DataFrame,
    trace_col: str,
    graphs_dir: Path,
) -> tuple[str, nx.DiGraph, dict, bool, bool]:
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

                # Extract and visualize call sequences for each parent node
                # Focus on nodes that have multiple children (interesting fanout patterns)
                parent_nodes = [n for n in G_user.nodes() if G_user.out_degree(n) >= 2]

                if parent_nodes:
                    # Create subdirectory for call sequences
                    seq_dir = service_dir / "call_sequences"
                    seq_dir.mkdir(exist_ok=True)

                    logger.debug(f"Service {service_name}: Extracting call sequences for {len(parent_nodes)} parent nodes")

                    # Collect all parent sequences for unified visualization
                    all_parent_sequences = {}

                    for parent in parent_nodes:
                        # Extract call sequences from each trace
                        sequences = []

                        for trace_id in unique_traces:
                            trace_df = service_df[service_df[trace_col] == trace_id]
                            trace_timing = _extract_timing_from_trace(trace_df, G_user)

                            # Only process if this parent appears in this trace
                            parent_appears = any(p == parent for (p, c) in trace_timing.keys())
                            if parent_appears:
                                sequence = _extract_call_sequence_from_trace(trace_timing, parent)
                                if sequence:
                                    sequences.append(sequence)

                        # Aggregate sequences across traces
                        if sequences:
                            call_sequence = _aggregate_call_sequences(sequences, len(unique_traces))

                            if call_sequence:
                                # Log the call sequence pattern
                                logger.info(f"  Parent '{parent}' call sequence ({len(sequences)} traces):")
                                for stage_idx, stage_probs in enumerate(call_sequence):
                                    children_str = ", ".join([f"{child}({prob:.2f})" for child, prob in sorted(stage_probs.items(), key=lambda x: x[1], reverse=True)])
                                    logger.info(f"    Stage {stage_idx + 1}: [{children_str}]")

                                # Draw individual call sequence diagram
                                parent_slug = _slugify(parent)
                                output_path = seq_dir / f"sequence_{parent_slug}.png"
                                _draw_call_sequence_graph(parent, call_sequence, output_path)

                                # Add to collection for unified diagram
                                all_parent_sequences[parent] = call_sequence

                    # Draw unified call sequence diagram showing all parents
                    if all_parent_sequences:
                        output_path = service_dir / "unified_call_sequences.png"
                        _draw_unified_call_sequence_graph(all_parent_sequences, output_path, service_name)
                        logger.info(f"  Generated unified call sequence diagram with {len(all_parent_sequences)} parents")

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
