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
) -> tuple[str, nx.DiGraph, dict]:
    """
    Process a single service: extract edges, create graph, and save visualization.
    
    Args:
        service_name: Name of the service
        service_df: DataFrame containing rows for this service
        trace_col: Column name containing trace IDs
        graphs_dir: Directory to save graph visualizations
    
    Returns:
        Tuple of (service_name, graph, statistics_dict)
    """
    # Get unique traces for this service
    unique_traces = service_df[trace_col].dropna().unique()
    
    if len(unique_traces) == 0:
        logger.warning(f"No valid traces found for service {service_name}")
        G = nx.DiGraph()
        stats = _compute_graph_statistics(G)
        return (service_name, G, stats)
    
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
    
    # Compute statistics
    stats = _compute_graph_statistics(G)
    
    # Create visualization
    if G.number_of_nodes() > 0:
        plt.figure(figsize=(14, 10))
        pos = _hierarchical_layout(G, ranksep=2.0, nodesep=0.8)
        
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
        
        plt.title(f"Aggregated Call Graph for Service '{service_name}'\n(Edge thickness and labels indicate frequency)", 
                  fontsize=16, fontweight='bold')
        plt.axis('off')
        plt.tight_layout()
        
        # Save graph
        output_path = graphs_dir / f"call_graph_service_{service_name}.png"
        plt.savefig(output_path, dpi=150, bbox_inches='tight')
        plt.close()
    
    return (service_name, G, stats)

def _process_service_wrapper(args: tuple) -> tuple[str, nx.DiGraph, dict]:
    """
    Wrapper function for parallel processing of services.
    Extracts arguments from tuple for ProcessPoolExecutor compatibility.
    
    Args:
        args: Tuple of (service_name, service_df, trace_col, graphs_dir_str)
              where graphs_dir_str is a string path that will be converted to Path
    
    Returns:
        Tuple of (service_name, graph, statistics_dict)
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
    logger.info(f"\n{'='*80}")
    logger.info("Grouping dataset by service")
    logger.info(f"{'='*80}")
    
    service_groups = df.groupby("service")
    service_names = list(service_groups.groups.keys())
    
    if len(service_names) == 0:
        logger.warning("No services found in dataset")
        return
    
    logger.info(f"Found {len(service_names)} service(s)")
    
    # Count traces per service and select top N
    logger.info(f"\nCounting traces per service...")
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
                _, G, stats = fut.result()
                stats["service_name"] = service_name
                all_stats.append(stats)
            except Exception as e:
                logger.error(f"Failed to process service {service_name}: {e!r}")
                raise
    
    # Sort statistics by number of nodes (descending)
    all_stats.sort(key=lambda x: x["num_nodes"], reverse=True)
    
    # Print statistics
    logger.info(f"\n{'='*80}")
    logger.info("Graph Statistics (sorted by number of nodes, descending)")
    logger.info(f"{'='*80}")
    logger.info(f"{'Service':<20} {'Nodes':<8} {'Out-Degree':<30} {'In-Degree':<30}")
    logger.info(f"{'':-<20} {'':-<8} {'':-<30} {'':-<30}")
    logger.info(f"{'':<20} {'':<8} {'Avg':<10} {'Min':<10} {'Max':<10} {'Avg':<10} {'Min':<10} {'Max':<10}")
    logger.info(f"{'':-<20} {'':-<8} {'':-<30} {'':-<30}")
    
    for stats in all_stats:
        service_name = stats["service_name"]
        num_nodes = stats["num_nodes"]
        out_avg = stats["out_degree_avg"]
        out_min = stats["out_degree_min"]
        out_max = stats["out_degree_max"]
        in_avg = stats["in_degree_avg"]
        in_min = stats["in_degree_min"]
        in_max = stats["in_degree_max"]
        
        logger.info(
            f"{service_name:<20} {num_nodes:<8} "
            f"{out_avg:<10.2f} {out_min:<10} {out_max:<10} "
            f"{in_avg:<10.2f} {in_min:<10} {in_max:<10}"
        )
    
    logger.info(f"\n{'='*80}")
    logger.info(f"Generated {len(all_stats)} graph(s) in {graphs_dir}")

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
    analyze_call_graphs(df, top_n=top_services, n_workers=args.workers)

if __name__ == "__main__":
    main()
