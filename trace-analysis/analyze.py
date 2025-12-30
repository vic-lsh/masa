#!/usr/bin/env python
# coding: utf-8
from __future__ import annotations

from networkx.classes.digraph import DiGraph


# Headless/parallel-safe plotting
import matplotlib
matplotlib.use("Agg")

from pathlib import Path
import copy
import numpy as np
import pandas as pd
import matplotlib.pyplot as plt
import networkx as nx
from concurrent.futures import ProcessPoolExecutor, as_completed
from functools import partial
import re
import json
import time
import argparse
import logging
from collections import Counter, defaultdict
from tqdm import tqdm

# Set up logger
logger = logging.getLogger(__name__)

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
        for fut in tqdm(as_completed(futures), total=len(futures), desc="Reading CSVs"):
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

def _slugify(name: str) -> str:
    s = re.sub(r"[^\w\-]+", "_", name.strip())
    s = re.sub(r"_+", "_", s).strip("_")
    return s or "service"

# ----------------------------
# Data loading & filtering
# ----------------------------

def load_concat_datasets(max_dataset: int) -> pd.DataFrame:
    return read_csvs_parallel(
        [get_csv_path(i) for i in range(max_dataset + 1)],
        on_bad_lines="skip",
    )

def filter_unknowns(df: pd.DataFrame) -> pd.DataFrame:
    df = df[df["um"].isin(["UNKNOWN", "UNAVAILABLE"]) == False]
    df = df[df["dm"].isin(["UNKNOWN", "UNAVAILABLE"]) == False]
    return df

def select_rpc_rows(df: pd.DataFrame) -> pd.DataFrame:
    return df[~df["rpctype"].isin(["UNKNOWN", "mq"])]

# ----------------------------
# Metrics
# ----------------------------

def print_rpc_stats(rpc_df: pd.DataFrame, df: pd.DataFrame) -> None:
    logger.info(f"Number of RPC calls: {len(rpc_df)}")
    logger.info(f"Number of total calls: {len(df)}")
    logger.info(f"Fraction of RPC calls: {len(rpc_df) / len(df)}")

    num_user_facing_svcs = len(rpc_df["service"].unique())
    logger.info(f"Number of unique user-facing services: {num_user_facing_svcs}")

    num_services = pd.concat([rpc_df["um"], rpc_df["dm"]]).nunique()
    logger.info(f"Number of unique microservices (not instances): {num_services}")

    num_instances = pd.concat([rpc_df["uminstanceid"], rpc_df["dminstanceid"]]).nunique()
    logger.info(f"Number of unique microservice instances: {num_instances}")

    avg_replica_count = num_instances / num_services
    logger.info(f"Average replica count per microservice: {avg_replica_count}")

def get_top_services(rpc_df: pd.DataFrame, n: int = 10) -> pd.Series:
    top_services = rpc_df["service"].value_counts().head(n)
    logger.info(f"Top {n} most popular services:")
    logger.info(f"\n{top_services}")
    return top_services

# ----------------------------
# Graph building & plotting
# ----------------------------

def get_service_graphs(df: pd.DataFrame, service_name: str, interface_col: str = "interface") -> tuple[DiGraph, DiGraph]:
    svc_df = df[df["service"] == service_name]
    iface_series = svc_df[interface_col].fillna("<none>")

    G_pair = nx.DiGraph()
    G_iface = nx.DiGraph()

    for (_, row), iface in zip(svc_df.iterrows(), iface_series):
        caller = row["um"]
        callee = row["dm"]
        dm_iface_node = (callee, iface)

        if G_pair.has_edge(caller, callee):
            G_pair[caller][callee]["weight"] += 1
            hist = G_pair[caller][callee]["interface_counts"]
            hist[iface] = hist.get(iface, 0) + 1
        else:
            G_pair.add_edge(
                caller,
                callee,
                weight=1,
                interface_counts={iface: 1},
            )

        if G_iface.has_edge(caller, dm_iface_node):
            G_iface[caller][dm_iface_node]["weight"] += 1
        else:
            G_iface.add_edge(caller, dm_iface_node, weight=1)

    return G_pair, G_iface

def plot_dag_plot(
    G: nx.DiGraph,
    mode: str = "thickness",
    outfile: Path | None = None,
    min_width: float = 0.8,
    max_width: float = 6.0,
    uniform_width: float = 2.0,
    ranksep: float = 2.0,
    nodesep: float = 0.8,
    node_size: int = 3000,
    font_size: int = 10,
    arrowsize: int = 20,
    node_color: str = "lightblue",
    figsize=(8, 6),
) -> None:
    pos = nx.nx_agraph.graphviz_layout(
        G, prog="dot", args=f"-Granksep={ranksep} -Gnodesep={nodesep}"
    )

    if mode == "thickness":
        raw_weights = []
        for u, v in G.edges():
            w = G[u][v].get("weight", 1.0)
            if w <= 0:
                w = 1e-6
            raw_weights.append(w)

        log_w = np.log1p(raw_weights)
        if np.ptp(log_w) == 0:
            norm = np.ones_like(log_w)
        else:
            norm = (log_w - log_w.min()) / np.ptp(log_w)
        widths = min_width + norm * (max_width - min_width)

        plt.figure(figsize=figsize)
        nx.draw(
            G, pos,
            with_labels=True, arrows=True,
            node_size=node_size, font_size=font_size,
            node_color=node_color, arrowsize=arrowsize,
            width=widths,
        )

    elif mode == "labels":
        widths = [uniform_width] * G.number_of_edges()
        edge_labels = nx.get_edge_attributes(G, "weight")

        plt.figure(figsize=figsize)
        nx.draw(
            G, pos,
            with_labels=True, arrows=True,
            node_size=node_size, font_size=font_size,
            node_color=node_color, arrowsize=arrowsize,
            width=widths,
        )
        if edge_labels:
            nx.draw_networkx_edge_labels(G, pos, edge_labels=edge_labels)
    else:
        raise ValueError("mode must be 'thickness' or 'labels'")

    if outfile:
        outfile.parent.mkdir(parents=True, exist_ok=True)
        plt.savefig(outfile, bbox_inches='tight')
        plt.close()
    else:
        plt.tight_layout()
        plt.show()

def reachable_subgraph(G: nx.DiGraph, source: str = "USER") -> nx.DiGraph:
    if source not in G:
        raise ValueError(f"Source node {source!r} not found in graph.")

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

# ----------------------------
# CallGraph wrapper
# ----------------------------

class CallGraph:
    service_name: str
    G_pair: nx.DiGraph
    G_iface: nx.DiGraph

    def __init__(self, service_name: str, G_pair: nx.DiGraph, G_iface: nx.DiGraph) -> None:
        self.service_name = service_name
        self.G_pair = G_pair
        self.G_iface = G_iface

    def draw_svc_plot(self, outfile: Path | None = None, **kwargs) -> None:
        plot_dag_plot(self.G_pair, outfile=outfile, **kwargs)

    def draw_dag(self, outfile: Path | None = None, **kwargs) -> None:
        plot_dag_plot(reachable_subgraph(self.G_pair, source="USER"), outfile=outfile, **kwargs)

# ----------------------------
# Latency distributions
# ----------------------------

def compute_latency_distributions(df: pd.DataFrame):
    return (
        df.groupby(["dm", "interface"])["rt"]
        .agg(list)
        .to_dict()
    )

def query_latency_distribution(latency_dict, dm_name: str, iface_name: str):
    return latency_dict.get((dm_name, iface_name), [])

# ----------------------------
# Reporting (per-graph)
# ----------------------------

def report_latency_by_edge_for_graph(
    graph: CallGraph,
    latency_dists: dict,
    output_root: str | Path = "graph_reports",
) -> None:
    output_root = Path(output_root)
    output_root.mkdir(parents=True, exist_ok=True)

    base = list(range(1, 101))
    tails = [99.5, 99.9, 99.95, 99.99]
    percentiles = sorted(set(base + tails))

    edges_rows: list[tuple[str, str, str, float]] = []
    iface_counts_by_callee: dict[tuple[str, str], Counter[str]] = defaultdict(Counter)
    seen_callee_ifaces: set[tuple[str, str]] = set()

    for caller, callee, data in reachable_subgraph(graph.G_pair, source="USER").edges(data=True):
        raw_weight = data.get("weight")
        weight = float(raw_weight) if raw_weight is not None else 0.0
        edges_rows.append((graph.service_name, caller, callee, weight))
        iface_counts = data.get("interface_counts", {}) or {}
        if iface_counts:
            iface_counts_by_callee[(graph.service_name, callee)].update(iface_counts)
            for iface in iface_counts.keys():
                seen_callee_ifaces.add((callee, iface))

    svc_dir = output_root / _slugify(graph.service_name)
    svc_dir.mkdir(parents=True, exist_ok=True)

    # (1) edges.csv
    if edges_rows:
        df_edges = pd.DataFrame(
            edges_rows,
            columns=pd.Index(["service", "caller", "callee", "weight"]),
        ).drop_duplicates()
        df_edges.to_csv(svc_dir / "edges.csv", index=False)

    # (2) interface_distribution.json — EXACT SHAPE: { [callee]: { [interface]: [call_count] } }
    iface_json: dict[str, dict[str, int]] = {}
    for (_service, callee), ctr in iface_counts_by_callee.items():
        if not ctr:
            continue
        callee_map = iface_json.setdefault(callee, {})
        for iface, cnt in ctr.items():
            callee_map[iface] = int(cnt)

    if iface_json:
        with open(svc_dir / "interface_distribution.json", "w") as f:
            json.dump(iface_json, f, indent=2)

    # (3) latency_percentiles.json
    lat_json: dict[str, dict[str, dict[str, float]]] = {}
    for (callee, iface) in sorted(seen_callee_ifaces):
        latencies = query_latency_distribution(latency_dists, callee, iface)
        if not latencies:
            continue
        arr = np.asarray(latencies, dtype=float)
        vals = np.percentile(arr, percentiles)
        lat_json.setdefault(callee, {})[iface] = {str(p): float(v) for p, v in zip(percentiles, vals)}

    if lat_json:
        with open(svc_dir / "latency_percentiles.json", "w") as f:
            json.dump(lat_json, f, indent=2)
# ----------------------------
# Per-service worker (PROCESS)
# ----------------------------

def _process_one_service_proc(
    service_name: str,
    svc_df_min: pd.DataFrame,
    latency_dists_filtered: dict,
    plots_outdir: Path,
    reports_root: Path,
) -> tuple[str, int, int]:
    """
    Build graphs for one service, draw plots, and write reports.
    Runs in a separate process. Returns (service, num_nodes, num_edges).
    """
    logger.info(f"Processing service {service_name!r} in process.")
    # Build graphs from the minimal per-service slice
    G_pair, G_iface = get_service_graphs(svc_df_min, service_name)
    cg = CallGraph(service_name, G_pair, G_iface)

    # Plots
    plots_outdir.mkdir(parents=True, exist_ok=True)
    cg.draw_svc_plot(
        mode="labels", figsize=(32, 12),
        outfile=plots_outdir / f"{_slugify(service_name)}_svc.png",
    )
    cg.draw_dag(
        mode="thickness", figsize=(20, 8),
        outfile=plots_outdir / f"{_slugify(service_name)}_dag.png",
    )

    # Reports
    report_latency_by_edge_for_graph(cg, latency_dists_filtered, output_root=reports_root)

    return (service_name, len(cg.G_pair.nodes), len(cg.G_pair.edges))

# ----------------------------
# Orchestration (process pool)
# ----------------------------

def run_for_services_process_pool(
    rpc_df: pd.DataFrame,
    top_services: pd.Series,
    n_workers: int | None = None,
    plots_outdir: Path = Path("plots"),
    reports_root: Path = Path("graph_reports"),
) -> list[tuple[str, int, int]]:
    """
    Parallelizes the per-service work (graphs, plots, reports) over top_services
    using a **ProcessPoolExecutor**. To reduce IPC overhead:
      * The parent builds a minimal per-service dataframe slice (only needed columns).
      * The parent computes global latency distributions once, then filters that dict
        per service to just the (dm, interface) keys used by that service.
    """
    plots_outdir.mkdir(parents=True, exist_ok=True)
    reports_root.mkdir(parents=True, exist_ok=True)

    # Precompute global latency distributions once
    latency_dists_all = compute_latency_distributions(rpc_df)

    # Prepare minimal per-service data and the subset of latency dict each service needs
    services = list(top_services.index)

    # Only columns required to build graphs (+ service for get_service_graphs filter)
    needed_cols = ["service", "um", "dm", "interface"]
    # Optional: keep "rt" out of svc_df_min (not required by workers—latency comes from latency_dists)
    svc_frames: dict[str, pd.DataFrame] = {
        svc: rpc_df.loc[rpc_df["service"] == svc, needed_cols].copy()
        for svc in services
    }

    # Precompute (dm, interface) pairs per service to trim latency dict size
    svc_keys: dict[str, set[tuple[str, str]]] = {
        svc: set(zip(svc_frames[svc]["dm"], svc_frames[svc]["interface"].fillna("<none>")))
        for svc in services
    }

    def _filter_latency(lat_all: dict, keys: set[tuple[str, str]]) -> dict:
        if not keys:
            return {}
        return {k: v for k, v in lat_all.items() if k in keys}

    results: list[tuple[str, int, int]] = []

    with ProcessPoolExecutor(max_workers=n_workers) as ex:
        futures = {}
        for svc in services:
            lat_sub = _filter_latency(latency_dists_all, svc_keys[svc])
            # Submit minimal payload to the process
            futures[ex.submit(
                _process_one_service_proc,
                svc,
                svc_frames[svc],
                lat_sub,
                plots_outdir,
                reports_root,
            )] = svc

        for fut in tqdm(as_completed(futures), total=len(futures), desc="Per-service (proc)"):
            svc = futures[fut]
            try:
                results.append(fut.result())
            except Exception as e:
                logger.warning(f"Service {svc} failed: {e!r}")

    results.sort(key=lambda x: x[0])
    return results

# ----------------------------
# main()
# ----------------------------

def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - %(name)s - %(levelname)s - %(message)s"
    )
    
    parser = argparse.ArgumentParser(
        description="Analyze microservice call graph traces from Alibaba cluster data"
    )
    parser.add_argument(
        "-n", "--num-datasets",
        type=int,
        default=10,
        help="Number of datasets to load (default: 10). Loads datasets 0 through (n-1)."
    )
    args = parser.parse_args()
    num_datasets = args.num_datasets
    
    if num_datasets < 1:
        parser.error("Number of datasets must be at least 1")
    
    # Convert number of datasets to max dataset ID (0-indexed)
    max_dataset = num_datasets - 1

    # Load & concat
    df = load_concat_datasets(max_dataset)

    # Filter unknowns, then select RPC rows
    df = filter_unknowns(df)
    rpc_df = select_rpc_rows(df)

    # Print stats & top services
    print_rpc_stats(rpc_df, df)
    top_services = get_top_services(rpc_df, n=50)

    # Set output directories relative to trace-analysis directory
    trace_analysis_dir = Path(__file__).parent.resolve()
    plots_outdir = trace_analysis_dir / "plots"
    reports_root = trace_analysis_dir / "graph_reports"
    
    start = time.perf_counter()
    results = run_for_services_process_pool(
        rpc_df,
        top_services,
        n_workers=32,               # set an int to cap processes
        plots_outdir=plots_outdir,
        reports_root=reports_root,
    )
    elapsed = time.perf_counter() - start
    logger.info(f"Elapsed: {elapsed:.6f} s")

    for svc, n_nodes, n_edges in results:
        logger.info(f"{svc}: nodes={n_nodes}, edges={n_edges}")

if __name__ == "__main__":
    main()