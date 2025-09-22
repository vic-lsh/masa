#!/usr/bin/env python
# coding: utf-8

from pathlib import Path
import copy
import numpy as np
import pandas as pd
import matplotlib.pyplot as plt
import networkx as nx


# ----------------------------
# Paths & basic utilities
# ----------------------------

PROJECT_HOME = Path("..").resolve()
CSV_PATH = (
    PROJECT_HOME
    / "traces"
    / "alibaba"
    / "cluster-trace-microservices-v2022"
    / "data"
    / "CallGraph"
    / "CallGraph_0.csv"
)

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

def load_concat_datasets(max_dataset: int) -> pd.DataFrame:
    dfs = []
    for i in range(max_dataset):
        dfs.append(pd.read_csv(get_csv_path(i), on_bad_lines="skip"))
    return pd.concat(dfs)

def filter_unknowns(df: pd.DataFrame) -> pd.DataFrame:
    df = df[df["um"] != "UNKNOWN"]
    df = df[df["dm"] != "UNKNOWN"]
    df = df[df["um"] != "UNAVAILABLE"]
    df = df[df["dm"] != "UNAVAILABLE"]
    return df

def select_rpc_rows(df: pd.DataFrame) -> pd.DataFrame:
    # Ignore unknown calls and message queue calls (not supported by our system)
    return df[~df["rpctype"].isin(["UNKNOWN", "mq"])]


# ----------------------------
# Metrics & prints
# ----------------------------

def print_basic_stats(df: pd.DataFrame) -> None:
    print(len(df))
    print(df["rpctype"].unique())

def print_rpc_stats(rpc_df: pd.DataFrame, df: pd.DataFrame) -> None:
    print("Number of RPC calls:", len(rpc_df))
    print("Number of total calls:", len(df))
    print("Fraction of RPC calls:", len(rpc_df) / len(df))

    num_user_facing_svcs = len(rpc_df["service"].unique())
    print("Number of unique user-facing services:", num_user_facing_svcs)

    num_services = pd.concat([rpc_df["um"], rpc_df["dm"]]).nunique()
    print("Number of unique microservices (not instances):", num_services)

    num_instances = pd.concat([rpc_df["uminstanceid"], rpc_df["dminstanceid"]]).nunique()
    print("Number of unique microservice instances:", num_instances)

    avg_replica_count = num_instances / num_services
    print("Average replica count per microservice:", avg_replica_count)

def top_10_services(rpc_df: pd.DataFrame) -> pd.Series:
    top_services = rpc_df["service"].value_counts().head(10)
    print("Top 10 most popular services:")
    print(top_services)
    return top_services


# ----------------------------
# Graph building & plotting
# ----------------------------

def get_service_graphs(df: pd.DataFrame, service_name: str, interface_col: str = "interface"):
    """
    Build two service graphs for a given `service_name` from `df`.

    Returns
    -------
    (G_pair, G_iface) : tuple(networkx.DiGraph, networkx.DiGraph)
        G_pair: edges (um -> dm) with attributes:
            - weight : int (total calls)
            - interface_counts : dict(interface -> int)
        G_iface: edges (um -> (dm, interface)) with attribute:
            - weight : int (total calls)
    """
    svc_df = df[df["service"] == service_name]
    iface_series = svc_df[interface_col].fillna("<none>")

    G_pair = nx.DiGraph()
    G_iface = nx.DiGraph()

    for (_, row), iface in zip(svc_df.iterrows(), iface_series):
        caller = row["um"]
        callee = row["dm"]
        dm_iface_node = (callee, iface)  # structured tuple node

        # Graph 1: (um -> dm)
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

        # Graph 2: (um -> (dm, interface))
        if G_iface.has_edge(caller, dm_iface_node):
            G_iface[caller][dm_iface_node]["weight"] += 1
        else:
            G_iface.add_edge(caller, dm_iface_node, weight=1)

    return G_pair, G_iface

def get_service_graph(df: pd.DataFrame, service_name: str):
    G = nx.DiGraph()
    svc_df = df[df["service"] == service_name]
    for _, row in svc_df.iterrows():
        caller = row["um"]
        callee = row["dm"]
        if G.has_edge(caller, callee):
            G[caller][callee]["weight"] += 1
        else:
            G.add_edge(caller, callee, weight=1)
    return G

def plot_dag_plot(
    G: nx.DiGraph,
    mode: str = "thickness",           # "thickness" or "labels"
    outfile: Path | None = None,       # if provided, save to file instead of showing
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
):
    """
    Plot a DAG with Graphviz 'dot' layout.

    mode="thickness": edges use log-scaled thickness from 'weight' (no edge labels)
    mode="labels":    edges use uniform thickness and display 'weight' as labels (pre-log)
    """
    pos = nx.nx_agraph.graphviz_layout(
        G, prog="dot", args=f"-Granksep={ranksep} -Gnodesep={nodesep}"
    )

    if mode == "thickness":
        raw_weights = []
        for u, v in G.edges():
            w = G[u][v].get("weight", 1.0)
            if w <= 0:
                w = 1e-6  # avoid log(0) / negatives
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

    plt.tight_layout()
    if outfile:
        outfile.parent.mkdir(parents=True, exist_ok=True)
        plt.savefig(outfile)
        plt.close()
    else:
        plt.show()

def reachable_subgraph(G: nx.DiGraph, source: str = "USER") -> nx.DiGraph:
    """
    Return a DiGraph induced by all nodes reachable from `source`
    (including `source`). Works with cycles and self-edges.
    """
    if source not in G:
        raise ValueError(f"Source node {source!r} not found in graph.")

    reachable = {source} | nx.descendants(G, source)
    view = G.subgraph(reachable)

    H = G.__class__()                     # preserve DiGraph/MultiDiGraph
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
    _reachable_subgraph: nx.DiGraph | None

    def __init__(self, service_name: str, G_pair: nx.DiGraph, G_iface: nx.DiGraph):
        self.service_name = service_name
        self.G_pair = G_pair
        self.G_iface = G_iface
        self._reachable_subgraph = None

    def draw_svc_plot(self, outfile: Path | None = None, **kwargs):
        """Draw a plot with all the service nodes included."""
        plot_dag_plot(self.G_pair, outfile=outfile, **kwargs)

    def draw_dag(self, outfile: Path | None = None, **kwargs):
        """Draw a plot with only nodes reachable from 'USER'."""
        plot_dag_plot(self.reachable_subgraph(), outfile=outfile, **kwargs)

    def reachable_subgraph(self, source: str = "USER") -> nx.DiGraph:
        """Get the reachable subgraph from `source`."""
        return reachable_subgraph(self.G_pair, source=source)


# ----------------------------
# Latency distributions
# ----------------------------

def compute_latency_distributions(df: pd.DataFrame):
    """
    Precompute latency distributions for each (dm, interface) pair.
    Returns: dict[(dm, interface)] -> list of rt
    """
    latency_groups = (
        df.groupby(["dm", "interface"])["rt"]
        .apply(list)
        .to_dict()
    )
    return latency_groups

def query_latency_distribution(latency_dict, dm_name: str, iface_name: str):
    return latency_dict.get((dm_name, iface_name), [])


# ----------------------------
# Orchestration
# ----------------------------

def build_call_graphs_for_top_services(rpc_df: pd.DataFrame, top_services: pd.Series) -> list[CallGraph]:
    graphs = []
    for svc in top_services.index:
        print(f"Service: {svc}")
        G_svc, G_iface = get_service_graphs(rpc_df, svc)
        graphs.append(CallGraph(svc, G_svc, G_iface))
    return graphs

def draw_and_report_graphs(graphs: list[CallGraph], outdir: Path) -> None:
    for call_graph in graphs:
        print("Service:", call_graph.service_name)
        print(
            "Number of nodes:",
            len(call_graph.G_pair.nodes),
            "Number of edges:",
            len(call_graph.G_pair.edges),
        )
        call_graph.draw_svc_plot(
            mode="labels", figsize=(32, 12),
            outfile=outdir / f"{call_graph.service_name}_svc.png"
        )
        call_graph.draw_dag(
            mode="thickness", figsize=(20, 8),
            outfile=outdir / f"{call_graph.service_name}_dag.png"
        )

def report_latency_by_edge(graphs: list[CallGraph], latency_dists: dict) -> None:
    for graph in graphs:
        print("\n\n\n---------------------------------")
        print("Service:", graph.service_name)

        for caller, callee, data in graph.reachable_subgraph().edges(data=True):
            iface_counts = data.get("interface_counts", {})
            n_interfaces = len(iface_counts)
            print("Data from edge:", (caller, callee), "number of interfaces:", n_interfaces)

            for iface, count in iface_counts.items():
                latencies = query_latency_distribution(latency_dists, callee, iface)
                if latencies:
                    print(
                        f"  Edge {caller} -> ({callee}, {iface}): {count} calls, "
                        f"latencies (ms): min={min(latencies):.2f}, "
                        f"max={max(latencies):.2f}, "
                        f"mean={sum(latencies)/len(latencies):.2f}, "
                        f"p99={np.percentile(latencies, 99):.2f}"
                    )
                else:
                    print(
                        f"  Edge {caller} -> ({callee}, {iface}): {count} calls, "
                        f"No latency data available."
                    )


# ----------------------------
# main()
# ----------------------------

def main():
    # Mirror the original print of CSV_PATH
    print(CSV_PATH)

    # Original parameter
    max_dataset = 2

    # Load & concat
    df = load_concat_datasets(max_dataset)

    # Original basic stats
    print_basic_stats(df)

    # Filter unknowns, then select RPC rows
    df = filter_unknowns(df)
    rpc_df = select_rpc_rows(df)

    # Print stats & top services
    print_rpc_stats(rpc_df, df)
    top_services = top_10_services(rpc_df)

    # Build graphs for top services
    graphs = build_call_graphs_for_top_services(rpc_df, top_services)

    # Draw plots and print node/edge counts (saved to files)
    outdir = Path("plots")
    draw_and_report_graphs(graphs, outdir)

    # Latency distributions and per-edge reporting
    latency_dists = compute_latency_distributions(rpc_df)
    report_latency_by_edge(graphs, latency_dists)


if __name__ == "__main__":
    main()