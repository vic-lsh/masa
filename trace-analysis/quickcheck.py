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

def _read_one(path: str | Path, **read_csv_kwargs) -> pd.DataFrame:
    return pd.read_csv(path, **read_csv_kwargs)

def read_csvs_parallel(
    paths: list[str | Path],
    n_workers: int | None = None,
    show_errors: bool = True,
    **read_csv_kwargs,
) -> pd.DataFrame:
    paths = list(paths)
    read_fn = partial(_read_one, **read_csv_kwargs)
    dfs = []

    with ProcessPoolExecutor(max_workers=n_workers) as ex:
        futures = {ex.submit(read_fn, p): p for p in paths}
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

def load_concat_datasets(max_dataset: int, max_rows: int | None = None) -> pd.DataFrame:
    read_kwargs = {"on_bad_lines": "skip"}
    if max_rows is not None:
        read_kwargs["nrows"] = max_rows
    return read_csvs_parallel(
        [get_csv_path(i) for i in range(max_dataset + 1)],
        **read_kwargs,
    )

def sample_traces(df: pd.DataFrame, fraction: float, trace_col: str = "traceid", random_state: int | None = None) -> pd.DataFrame:
    """
    Sample a fraction of traces from the dataframe.
    
    For rows belonging to the same trace, either keep them all or drop them all
    (maintains trace integrity).
    
    Args:
        df: Input dataframe with trace data
        fraction: Fraction of traces to sample (0.0 to 1.0)
        trace_col: Column name containing trace IDs (default: "traceid")
        random_state: Random seed for reproducibility (default: None)
    
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
    
    # Filter dataframe to keep all rows for sampled traces
    sampled_df = df[df[trace_col].isin(sampled_trace_ids)].copy()
    
    logger.info(f"Sampled {len(sampled_trace_ids):,} traces ({fraction*100:.1f}%) from {len(unique_traces):,} unique traces")
    logger.info(f"Result: {len(sampled_df):,} rows from {len(df):,} original rows")
    
    return sampled_df

# ----------------------------
# Data cleaning
# ----------------------------

def clean_data(df: pd.DataFrame) -> pd.DataFrame:
    """
    Clean the dataframe by removing rows where 'um' or 'dm' equals 'UNKNOWN'.
    
    Args:
        df: Input dataframe with trace data
    
    Returns:
        Cleaned dataframe
    """
    original_len = len(df)
    
    # Remove rows where um or dm equals UNKNOWN
    if "um" in df.columns and "dm" in df.columns:
        cleaned_df = df[(df["um"] != "UNKNOWN") & (df["dm"] != "UNKNOWN")].copy()
    elif "um" in df.columns:
        cleaned_df = df[df["um"] != "UNKNOWN"].copy()
    elif "dm" in df.columns:
        cleaned_df = df[df["dm"] != "UNKNOWN"].copy()
    else:
        logger.warning("Columns 'um' or 'dm' not found, skipping UNKNOWN cleaning")
        return df
    
    removed = original_len - len(cleaned_df)
    if removed > 0:
        logger.info(f"Cleaned data: removed {removed:,} rows where 'um' or 'dm' equals UNKNOWN (from {original_len:,} to {len(cleaned_df):,} rows)")
    else:
        logger.info("No rows found with 'um' or 'dm' equal to UNKNOWN")
    
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

def analyze_call_graphs(df: pd.DataFrame, trace_col: str = "traceid") -> None:
    """
    Analyze call graphs from the dataframe.
    First step: print the first trace.
    
    Args:
        df: Input dataframe with trace data
        trace_col: Column name containing trace IDs (default: "traceid")
    """
    if trace_col not in df.columns:
        logger.warning(f"Column '{trace_col}' not found in dataframe. Available columns: {list(df.columns)}")
        return
    
    # Get unique trace IDs
    unique_traces = df[trace_col].dropna().unique()
    
    if len(unique_traces) == 0:
        logger.warning("No valid trace IDs found")
        return
    
    # Get the first trace
    trace_id = unique_traces[0]
    first_trace_df = df[df[trace_col] == trace_id].copy()
    
    logger.info(f"\n{'='*80}")
    logger.info(f"First trace (traceid: {trace_id})")
    logger.info(f"{'='*80}")
    logger.info(f"Number of rows in trace: {len(first_trace_df)}")
    logger.info(f"\nTrace data:")
    
    # Print the trace data in a readable format
    # Sort by timestamp if available, otherwise by index
    if "timestamp" in first_trace_df.columns:
        first_trace_df = first_trace_df.sort_values("timestamp")
    
    # Select key columns to display
    display_cols = []
    for col in ["traceid", "rpc_id", "service", "um", "dm", "interface", "timestamp", "rt", "rpctype"]:
        if col in first_trace_df.columns:
            display_cols.append(col)
    
    # Print all columns if we don't have the standard ones
    if not display_cols:
        display_cols = list(first_trace_df.columns)
    
    # Use pandas to_string for better formatting
    trace_display = first_trace_df[display_cols].to_string(index=False)
    logger.info(f"\n{trace_display}\n")
    
    # Compute edge list from rpc_id hierarchy
    if "rpc_id" not in first_trace_df.columns or "um" not in first_trace_df.columns or "dm" not in first_trace_df.columns:
        logger.warning("Missing required columns (rpc_id, um, dm) for graph computation")
        return
    
    # Create a mapping from rpc_id to dm (downstream microservice)
    # Since each call is recorded twice (in UM and DM), we need to get a consistent dm value
    # Convert rpc_id to string and group
    first_trace_df = first_trace_df.copy()
    first_trace_df["rpc_id_str"] = first_trace_df["rpc_id"].astype(str).str.strip()
    
    rpc_to_dm = {}
    rpc_groups = first_trace_df.groupby("rpc_id_str")
    
    for rpc_id, group in rpc_groups:
        if rpc_id and rpc_id != "" and rpc_id != "nan":
            # Get the most common dm value for this rpc_id (should be the same, but handle duplicates)
            dm_values = group["dm"].dropna().unique()
            if len(dm_values) > 0:
                # Use the first non-null dm value (they should all be the same)
                rpc_to_dm[rpc_id] = dm_values[0]
    
    # Build edge list
    edges = []
    nodes = set()
    
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
                logger.warning(f"Parent rpc_id {parent_rpc_id} not found for rpc_id {rpc_id}")
                continue
        
        edges.append((source, target))
        nodes.add(source)
        nodes.add(target)
    
    # Print edge list
    logger.info(f"\n{'='*80}")
    logger.info("Edge List (caller -> callee):")
    logger.info(f"{'='*80}")
    for source, target in sorted(edges):
        logger.info(f"  {source} -> {target}")
    logger.info(f"\nTotal edges: {len(edges)}")
    logger.info(f"Total nodes: {len(nodes)}")
    
    # Create and visualize graph
    G = nx.DiGraph()
    G.add_edges_from(edges)
    
    # Create visualization
    plt.figure(figsize=(12, 8))
    pos = nx.spring_layout(G, k=2, iterations=50)
    
    # Draw nodes
    nx.draw_networkx_nodes(G, pos, node_color='lightblue', node_size=2000, alpha=0.9)
    
    # Draw edges
    nx.draw_networkx_edges(G, pos, edge_color='gray', arrows=True, arrowsize=20, alpha=0.6)
    
    # Draw labels
    nx.draw_networkx_labels(G, pos, font_size=10, font_weight='bold')
    
    plt.title(f"Call Graph for Trace {trace_id}", fontsize=16, fontweight='bold')
    plt.axis('off')
    plt.tight_layout()
    
    # Save graph
    output_path = Path(__file__).parent / f"call_graph_trace_{trace_id}.png"
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()
    
    logger.info(f"\nGraph visualization saved to: {output_path}")

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
        help="Fraction of traces to sample (0.0 to 1.0, default: 1.0). Set to 1.0 to use all traces."
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
    args = parser.parse_args()
    num_datasets = args.num_datasets
    sample_fraction = args.sample_fraction
    random_state = args.random_state
    max_rows = args.max_rows
    
    if num_datasets < 1:
        parser.error("Number of datasets must be at least 1")
    
    if sample_fraction <= 0.0 or sample_fraction > 1.0:
        parser.error("Sample fraction must be in (0.0, 1.0]")
    
    if max_rows is not None and max_rows < 1:
        parser.error("Max rows must be at least 1")
    
    # Convert number of datasets to max dataset ID (0-indexed)
    max_dataset = num_datasets - 1

    # Load & concat
    logger.info(f"Loading {num_datasets} dataset(s) (datasets 0 through {max_dataset})")
    if max_rows is not None:
        logger.info(f"Limiting to {max_rows:,} rows per CSV file")
    df = load_concat_datasets(max_dataset, max_rows=max_rows)
    logger.info(f"Loaded {len(df)} rows from {num_datasets} dataset(s)")

    # Sample traces if requested
    if sample_fraction < 1.0:
        logger.info(f"Sampling {sample_fraction*100:.1f}% of traces (random_state={random_state})")
        df = sample_traces(df, fraction=sample_fraction, random_state=random_state)
    else:
        logger.info("Using all traces (no sampling)")

    # Clean data before analysis
    df = clean_data(df)

    # Analyze call graphs
    analyze_call_graphs(df)

if __name__ == "__main__":
    main()
