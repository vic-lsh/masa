#!/usr/bin/env python3

"""Find shared nodes between pairs of service directories and rank them."""

from __future__ import annotations

import argparse
import csv
import copy
from pathlib import Path
from typing import Dict, Set, Tuple

import networkx as nx
import matplotlib.pyplot as plt


def extract_nodes(edges_file: Path) -> Set[str]:
    """Extract all unique nodes (callers and callees) from an edges.csv file, excluding USER."""
    nodes: Set[str] = set()
    
    if not edges_file.exists():
        return nodes
    
    try:
        with edges_file.open(newline="") as csv_file:
            reader = csv.DictReader(csv_file)
            
            # Check if required columns exist
            if "caller" not in reader.fieldnames or "callee" not in reader.fieldnames:
                return nodes
            
            for row in reader:
                caller = row.get("caller", "").strip()
                callee = row.get("callee", "").strip()
                
                # Exclude USER node
                if caller and caller != "USER":
                    nodes.add(caller)
                if callee and callee != "USER":
                    nodes.add(callee)
    except Exception as e:
        print(f"Warning: Error reading {edges_file}: {e}", file=__import__("sys").stderr)
    
    return nodes


def find_shared_nodes(graphs_dir: Path) -> list[Tuple[int, str, int, str, int]]:
    """
    Find shared nodes between all pairs of service directories.
    
    Returns:
        List of tuples (shared_count, service1, total1, service2, total2) sorted by shared_count descending.
        total1 and total2 are the total node counts (excluding USER) for each service.
    """
    # Collect all service directories and their nodes
    service_nodes: Dict[str, Set[str]] = {}
    
    for service_dir in sorted(graphs_dir.glob("S_*/")):
        if not service_dir.is_dir():
            continue
        
        edges_file = service_dir / "edges.csv"
        if not edges_file.exists():
            continue
        
        service_name = service_dir.name
        nodes = extract_nodes(edges_file)
        
        if nodes:
            service_nodes[service_name] = nodes
    
    if len(service_nodes) < 2:
        return []
    
    # Compare all pairs of services
    results: list[Tuple[int, str, int, str, int]] = []
    service_list = sorted(service_nodes.keys())
    
    for i in range(len(service_list) - 1):
        service1 = service_list[i]
        nodes1 = service_nodes[service1]
        total1 = len(nodes1)
        
        for j in range(i + 1, len(service_list)):
            service2 = service_list[j]
            nodes2 = service_nodes[service2]
            total2 = len(nodes2)
            
            # Find shared nodes (intersection)
            shared_nodes = nodes1 & nodes2
            shared_count = len(shared_nodes)
            
            if shared_count > 0:
                results.append((shared_count, service1, total1, service2, total2))
    
    # Sort by shared count (descending), then by service names
    results.sort(key=lambda x: (-x[0], x[1], x[3]))
    
    return results


def build_graph_from_edges(edges_file: Path) -> nx.DiGraph:
    """Build a NetworkX graph from an edges.csv file."""
    G = nx.DiGraph()
    
    if not edges_file.exists():
        return G
    
    try:
        with edges_file.open(newline="") as csv_file:
            reader = csv.DictReader(csv_file)
            
            if "caller" not in reader.fieldnames or "callee" not in reader.fieldnames:
                return G
            
            for row in reader:
                caller = row.get("caller", "").strip()
                callee = row.get("callee", "").strip()
                weight = float(row.get("weight", 1.0))
                
                if caller and callee:
                    if G.has_edge(caller, callee):
                        G[caller][callee]["weight"] += weight
                    else:
                        G.add_edge(caller, callee, weight=weight)
    except Exception as e:
        print(f"Warning: Error reading {edges_file}: {e}", file=__import__("sys").stderr)
    
    return G


def merge_graphs(G1: nx.DiGraph, G2: nx.DiGraph) -> nx.DiGraph:
    """Merge two graphs, summing weights for edges that exist in both."""
    G_merged = nx.DiGraph()
    
    # Add all edges from G1
    for u, v, data in G1.edges(data=True):
        weight = data.get("weight", 1.0)
        if G_merged.has_edge(u, v):
            G_merged[u][v]["weight"] += weight
        else:
            G_merged.add_edge(u, v, weight=weight)
    
    # Add all edges from G2, summing weights if edge already exists
    for u, v, data in G2.edges(data=True):
        weight = data.get("weight", 1.0)
        if G_merged.has_edge(u, v):
            G_merged[u][v]["weight"] += weight
        else:
            G_merged.add_edge(u, v, weight=weight)
    
    return G_merged


def reachable_subgraph(G: nx.DiGraph, source: str = "USER") -> nx.DiGraph:
    """Extract the subgraph reachable from a source node."""
    if source not in G:
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


def _hierarchical_layout(G: nx.DiGraph, ranksep: float = 2.0, nodesep: float = 0.8) -> dict:
    """Create a hierarchical layout for a directed graph."""
    if G.number_of_nodes() == 0:
        return {}
    
    # Try Graphviz layout first
    try:
        pos = nx.nx_agraph.graphviz_layout(
            G, prog="dot", args=f"-Granksep={ranksep} -Gnodesep={nodesep}"
        )
        return pos
    except (ImportError, AttributeError, Exception):
        pass
    
    # Fallback: custom hierarchical layout
    roots = [n for n in G.nodes() if G.in_degree(n) == 0]
    if not roots:
        min_in_degree = min(G.in_degree(n) for n in G.nodes())
        roots = [n for n in G.nodes() if G.in_degree(n) == min_in_degree]
    
    node_depth = {}
    visited = set()
    queue = [(root, 0) for root in roots]
    
    while queue:
        node, depth = queue.pop(0)
        if node in visited:
            continue
        visited.add(node)
        node_depth[node] = depth
        
        for successor in G.successors(node):
            if successor not in visited:
                queue.append((successor, depth + 1))
    
    for node in G.nodes():
        if node not in node_depth:
            min_depth = float('inf')
            for root in roots:
                try:
                    path_length = nx.shortest_path_length(G, root, node)
                    min_depth = min(min_depth, path_length)
                except nx.NetworkXNoPath:
                    continue
            node_depth[node] = min_depth if min_depth != float('inf') else 0
    
    depth_groups = {}
    for node, depth in node_depth.items():
        if depth not in depth_groups:
            depth_groups[depth] = []
        depth_groups[depth].append(node)
    
    max_depth = max(depth_groups.keys()) if depth_groups else 0
    
    pos = {}
    for depth, nodes in depth_groups.items():
        y = max_depth - depth
        n_nodes = len(nodes)
        if n_nodes == 1:
            x_positions = [0.0]
        else:
            x_positions = [i / (n_nodes - 1) * 2 - 1 for i in range(n_nodes)]
        
        for node, x in zip(sorted(nodes), x_positions):
            pos[node] = (x, y)
    
    return pos


def _draw_graph(
    G: nx.DiGraph, 
    title: str, 
    output_path: Path, 
    num_nodes: int,
    node_membership: Dict[str, Set[str]] | None = None,
    service1_name: str | None = None,
    service2_name: str | None = None,
) -> None:
    """
    Draw a graph with dynamic sizing and save to file.
    
    Args:
        G: NetworkX graph to draw
        title: Title for the graph
        output_path: Path to save the image
        num_nodes: Number of nodes in the graph
        node_membership: Dictionary mapping node -> set of service names it belongs to
        service1_name: Name of first service (for legend)
        service2_name: Name of second service (for legend)
    """
    if G.number_of_nodes() == 0:
        return
    
    # Compute dynamic figure size
    if num_nodes < 50:
        base_width, base_height = 14.0, 10.0
        width_per_node, height_per_node = 0.4, 0.2
        min_width, min_height = 14.0, 10.0
        max_width, max_height = 40.0, 30.0
    elif num_nodes < 200:
        base_width, base_height = 16.0, 12.0
        width_per_node, height_per_node = 0.25, 0.15
        min_width, min_height = 16.0, 12.0
        max_width, max_height = 50.0, 35.0
    else:
        base_width, base_height = 18.0, 14.0
        width_per_node, height_per_node = 0.15, 0.1
        min_width, min_height = 18.0, 14.0
        max_width, max_height = 60.0, 40.0
    
    figsize = (
        min(max_width, max(min_width, base_width + num_nodes * width_per_node)),
        min(max_height, max(min_height, base_height + num_nodes * height_per_node))
    )
    
    # Auto-compute node_size, font_size, and spacing
    if num_nodes < 20:
        node_size, font_size, ranksep, nodesep = 3000, 10, 2.0, 0.8
    elif num_nodes < 50:
        node_size, font_size, ranksep, nodesep = 2800, 9, 1.8, 0.7
    elif num_nodes < 100:
        node_size, font_size, ranksep, nodesep = 2600, 8, 1.5, 0.6
    elif num_nodes < 200:
        node_size, font_size, ranksep, nodesep = 2400, 7, 1.2, 0.5
    else:
        node_size, font_size, ranksep, nodesep = 2200, 7, 1.0, 0.4
    
    plt.figure(figsize=figsize)
    pos = _hierarchical_layout(G, ranksep=ranksep, nodesep=nodesep)
    
    # Get edge weights for visualization
    edge_weights = [G[u][v].get('weight', 1) for u, v in G.edges()]
    max_weight = max(edge_weights) if edge_weights else 1
    min_weight = min(edge_weights) if edge_weights else 1
    
    # Normalize edge widths (min 1, max 5)
    edge_widths = [1 + 4 * (w - min_weight) / (max_weight - min_weight) if max_weight > min_weight else 3 
                   for w in edge_weights]
    
    # Color nodes based on membership
    if node_membership and service1_name and service2_name:
        node_colors = []
        for node in G.nodes():
            if node == "USER":
                # USER node is in both services - orange
                node_colors.append('#FFA500')  # Orange
            else:
                membership = node_membership.get(node, set())
                if service1_name in membership and service2_name in membership:
                    # Node in both services - orange
                    node_colors.append('#FFA500')  # Orange
                elif service1_name in membership:
                    # Node only in service 1 - light blue
                    node_colors.append('#87CEEB')  # Sky blue
                elif service2_name in membership:
                    # Node only in service 2 - darker blue
                    node_colors.append('#4682B4')  # Steel blue
                else:
                    # Unknown - gray
                    node_colors.append('#D3D3D3')  # Light gray
    else:
        # Default color if no membership info
        node_colors = ['lightblue'] * G.number_of_nodes()
    
    # Draw nodes with colors
    nx.draw_networkx_nodes(G, pos, node_color=node_colors, node_size=node_size, alpha=0.9)
    
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
    edge_labels = {(u, v): str(int(G[u][v].get('weight', 1))) for u, v in G.edges()}
    edge_label_font_size = max(6, font_size - 2)
    nx.draw_networkx_edge_labels(G, pos, edge_labels, font_size=edge_label_font_size)
    
    # Add legend if we have membership info
    if node_membership and service1_name and service2_name:
        from matplotlib.patches import Patch
        legend_elements = [
            Patch(facecolor='#87CEEB', label=f'{service1_name} only'),
            Patch(facecolor='#4682B4', label=f'{service2_name} only'),
            Patch(facecolor='#FFA500', label='Both services'),
        ]
        plt.legend(handles=legend_elements, loc='upper right', fontsize=9)
    
    plt.title(title, fontsize=16, fontweight='bold')
    plt.axis('off')
    plt.tight_layout()
    
    # Save graph
    output_path.parent.mkdir(parents=True, exist_ok=True)
    plt.savefig(output_path, dpi=150, bbox_inches='tight')
    plt.close()


def generate_report(
    service1: str,
    service2: str,
    G1_user: nx.DiGraph,
    G2_user: nx.DiGraph,
    G_union_user: nx.DiGraph,
    node_membership: Dict[str, Set[str]],
    output_path: Path,
) -> None:
    """Generate a report.txt file with statistics about the union graph."""
    # Count nodes by membership
    nodes_service1_only = [n for n in G_union_user.nodes() 
                           if node_membership.get(n, set()) == {service1}]
    nodes_service2_only = [n for n in G_union_user.nodes() 
                           if node_membership.get(n, set()) == {service2}]
    nodes_both = [n for n in G_union_user.nodes() 
                  if node_membership.get(n, set()) == {service1, service2}]
    
    # Count nodes in each service's USER subgraph (excluding USER node)
    G1_nodes = {n for n in G1_user.nodes() if n != "USER"}
    G2_nodes = {n for n in G2_user.nodes() if n != "USER"}
    G_union_nodes = {n for n in G_union_user.nodes() if n != "USER"}
    
    with output_path.open('w') as f:
        f.write(f"Union Graph Report: {service1} + {service2}\n")
        f.write("=" * 60 + "\n\n")
        
        f.write("Service Statistics (USER-reachable subgraphs, excluding USER node):\n")
        f.write("-" * 60 + "\n")
        f.write(f"{service1}: {len(G1_nodes)} nodes\n")
        f.write(f"{service2}: {len(G2_nodes)} nodes\n")
        f.write(f"Union: {len(G_union_nodes)} nodes\n\n")
        
        f.write("Node Overlap Analysis:\n")
        f.write("-" * 60 + "\n")
        f.write(f"Nodes in {service1} only: {len(nodes_service1_only)}\n")
        f.write(f"Nodes in {service2} only: {len(nodes_service2_only)}\n")
        f.write(f"Nodes in both services: {len(nodes_both)}\n")
        f.write(f"Total shared nodes: {len(nodes_both)}\n\n")
        
        if len(G_union_nodes) > 0:
            overlap_percentage = (len(nodes_both) / len(G_union_nodes)) * 100
            f.write(f"Overlap percentage: {overlap_percentage:.2f}%\n\n")
        
        f.write("Edge Statistics:\n")
        f.write("-" * 60 + "\n")
        f.write(f"{service1}: {G1_user.number_of_edges()} edges\n")
        f.write(f"{service2}: {G2_user.number_of_edges()} edges\n")
        f.write(f"Union: {G_union_user.number_of_edges()} edges\n\n")
        
        if nodes_both:
            f.write(f"Shared Nodes ({len(nodes_both)}):\n")
            f.write("-" * 60 + "\n")
            for node in sorted(nodes_both):
                f.write(f"  - {node}\n")
            f.write("\n")


def generate_union_graph(
    graphs_dir: Path,
    service1: str,
    service2: str,
    output_dir: Path,
) -> None:
    """Generate a union graph for two services and save it."""
    # Sort service names for consistent folder naming
    sorted_services = sorted([service1, service2])
    folder_name = f"{sorted_services[0]}_{sorted_services[1]}"
    union_dir = output_dir / folder_name
    union_dir.mkdir(parents=True, exist_ok=True)
    
    # Load graphs from both services
    edges1 = graphs_dir / service1 / "edges.csv"
    edges2 = graphs_dir / service2 / "edges.csv"
    
    G1 = build_graph_from_edges(edges1)
    G2 = build_graph_from_edges(edges2)
    
    if G1.number_of_nodes() == 0 and G2.number_of_nodes() == 0:
        print(f"Warning: Both services {service1} and {service2} have no edges", file=__import__("sys").stderr)
        return
    
    # Extract USER-reachable subgraphs for each service
    G1_user = reachable_subgraph(G1, source="USER")
    G2_user = reachable_subgraph(G2, source="USER")
    
    # Track node membership
    node_membership: Dict[str, Set[str]] = {}
    for node in G1_user.nodes():
        if node != "USER":  # Exclude USER from membership tracking
            node_membership.setdefault(node, set()).add(service1)
    for node in G2_user.nodes():
        if node != "USER":  # Exclude USER from membership tracking
            node_membership.setdefault(node, set()).add(service2)
    
    # Merge graphs
    G_merged = merge_graphs(G1, G2)
    
    # Extract USER-reachable subgraph from merged graph
    G_union_user = reachable_subgraph(G_merged, source="USER")
    
    if G_union_user.number_of_nodes() == 0:
        print(f"Warning: No USER-reachable nodes in union of {service1} and {service2}", file=__import__("sys").stderr)
        return
    
    # Generate report
    report_path = union_dir / "report.txt"
    generate_report(service1, service2, G1_user, G2_user, G_union_user, node_membership, report_path)
    
    # Generate graph visualization
    num_nodes = G_union_user.number_of_nodes()
    title = f"Union Call Graph: {service1} + {service2} (USER-Reachable Subgraph: {num_nodes} nodes)\n(Edge thickness and labels indicate frequency)"
    output_path = union_dir / "graph_user.png"
    
    _draw_graph(G_union_user, title, output_path, num_nodes, node_membership, service1, service2)
    print(f"Generated union graph for {service1} + {service2} -> {output_path}")
    print(f"Generated report for {service1} + {service2} -> {report_path}")


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Find shared nodes between pairs of service directories and rank them."
    )
    parser.add_argument(
        "--graphs-dir",
        type=Path,
        default=Path(__file__).parent / "graphs",
        help="Directory containing service directories (default: trace-analysis/graphs).",
    )
    args = parser.parse_args()
    
    if not args.graphs_dir.exists():
        print(f"Error: Directory {args.graphs_dir} does not exist", file=__import__("sys").stderr)
        return
    
    results = find_shared_nodes(args.graphs_dir)
    
    if not results:
        print("No shared nodes found between any service pairs.")
        return
    
    # Print header
    print(f"{'Shared':<10} {'Service 1':<20} {'Total 1':<10} {'Service 2':<20} {'Total 2':<10}")
    print("-" * 80)
    
    # Print results
    for shared_count, service1, total1, service2, total2 in results:
        print(f"{shared_count:<10} {service1:<20} {total1:<10} {service2:<20} {total2:<10}")
    
    # Generate union graphs for top 10 pairs
    graphs_union_dir = args.graphs_dir.parent / "graphs_union"
    graphs_union_dir.mkdir(parents=True, exist_ok=True)
    
    top_10 = results[:10]
    print(f"\nGenerating union graphs for top {len(top_10)} service pairs...")
    
    for shared_count, service1, total1, service2, total2 in top_10:
        generate_union_graph(args.graphs_dir, service1, service2, graphs_union_dir)
    
    print(f"\nUnion graphs saved to {graphs_union_dir}")


if __name__ == "__main__":
    main()
