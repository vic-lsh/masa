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

def analyze_call_graphs(df: pd.DataFrame, trace_col: str = "traceid") -> None:
    """
    Analyze call graphs by grouping by service and aggregating edges across all traces.
    For one service, compute the union of all edges and their frequencies, then plot.
    
    Args:
        df: Input dataframe with trace data
        trace_col: Column name containing trace IDs (default: "traceid")
    """
    # Check required columns
    required_cols = ["service", trace_col, "rpc_id", "um", "dm"]
    missing_cols = [col for col in required_cols if col not in df.columns]
    if missing_cols:
        logger.warning(f"Missing required columns: {missing_cols}. Available columns: {list(df.columns)}")
        return
    
    # Group by service
    logger.info(f"\n{'='*80}")
    logger.info("Grouping dataset by service")
    logger.info(f"{'='*80}")
    
    service_groups = df.groupby("service")
    service_names = list(service_groups.groups.keys())
    
    if len(service_names) == 0:
        logger.warning("No services found in dataset")
        return
    
    logger.info(f"Found {len(service_names)} service(s): {service_names}")
    
    # Select the first service (or could select by most traces)
    # selected_service = service_names[2]
    selected_service = "S_100071952"
    service_df = service_groups.get_group(selected_service).copy()
    
    logger.info(f"\nAnalyzing service: {selected_service}")
    logger.info(f"Rows for this service: {len(service_df):,}")
    
    # Get unique traces for this service
    unique_traces = service_df[trace_col].dropna().unique()
    logger.info(f"Number of traces for this service: {len(unique_traces):,}")
    
    if len(unique_traces) == 0:
        logger.warning(f"No valid traces found for service {selected_service}")
        return
    
    # Aggregate edges across all traces
    logger.info(f"\nExtracting edges from all traces...")
    edge_counter: dict[tuple[str, str], int] = {}
    
    for trace_id in tqdm(unique_traces, desc="Processing traces", file=sys.stderr):
        trace_df = service_df[service_df[trace_col] == trace_id]
        edges = _extract_edges_from_trace(trace_df)
        
        # Count edge frequencies
        for edge in edges:
            edge_counter[edge] = edge_counter.get(edge, 0) + 1
    
    # Sort edges by frequency (descending)
    sorted_edges = sorted(edge_counter.items(), key=lambda x: x[1], reverse=True)
    
    # Print aggregated edge list with frequencies
    logger.info(f"\n{'='*80}")
    logger.info(f"Aggregated Edge List for Service '{selected_service}' (caller -> callee, frequency):")
    logger.info(f"{'='*80}")
    for (source, target), frequency in sorted_edges:
        logger.info(f"  {source} -> {target} : {frequency}")
    
    logger.info(f"\nTotal unique edges: {len(edge_counter)}")
    logger.info(f"Total edge occurrences: {sum(edge_counter.values())}")
    
    # Create graph with edge weights
    G = nx.DiGraph()
    for (source, target), frequency in edge_counter.items():
        G.add_edge(source, target, weight=frequency)
    
    # Create visualization
    plt.figure(figsize=(14, 10))
    pos = nx.spring_layout(G, k=2, iterations=50)
    
    # Get edge weights for visualization
    edge_weights = [G[u][v]['weight'] for u, v in G.edges()]
    max_weight = max(edge_weights) if edge_weights else 1
    min_weight = min(edge_weights) if edge_weights else 1
    
    # Normalize edge widths (min 1, max 5)
    edge_widths = [1 + 4 * (w - min_weight) / (max_weight - min_weight) if max_weight > min_weight else 3 
                   for w in edge_weights]
    
    # Draw nodes
    nx.draw_networkx_nodes(G, pos, node_color='lightblue', node_size=2000, alpha=0.9)
    
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
    nx.draw_networkx_labels(G, pos, font_size=10, font_weight='bold')
    
    # Add edge labels with frequencies
    edge_labels = {(u, v): str(G[u][v]['weight']) for u, v in G.edges()}
    nx.draw_networkx_edge_labels(G, pos, edge_labels, font_size=8)
    
    plt.title(f"Aggregated Call Graph for Service '{selected_service}'\n(Edge thickness and labels indicate frequency)", 
              fontsize=16, fontweight='bold')
    plt.axis('off')
    plt.tight_layout()
    
    # Save graph
    output_path = Path(__file__).parent / f"call_graph_service_{selected_service}.png"
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
