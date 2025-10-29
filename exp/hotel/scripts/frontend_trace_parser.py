#!/usr/bin/env python3
"""Parse trace CSV files into JSON suitable for load replay."""

from __future__ import annotations

import argparse
import csv
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Sequence


def sum_span_latency(spans: Sequence[dict]) -> int:
    """Recursively sum latency across spans and their children."""
    total = 0
    for span in spans:
        total += int(span.get("latency_us", 0) or 0)
    return total

SPAN_PATTERN = re.compile(r"(?P<kind>\w+)\((?P<value>\d+)us\)")


@dataclass
class LocalSpan:
    """A span recorded on a single service without crossing process boundaries."""

    raw_kind: str
    latency_us: int

    def normalised_kind(self) -> str:
        return "Compute" if self.raw_kind == "Compute" else "Block"

    def to_trace_span(self) -> dict:
        return {"type": self.normalised_kind(), "latency_us": self.latency_us}


@dataclass
class ServiceNode:
    """A service invocation extracted from the trace."""

    name: str
    start_timestamp: int
    local_spans: List[LocalSpan] = field(default_factory=list)
    children: List["ServiceNode"] = field(default_factory=list)

    def total_latency_us(self) -> int:
        local = sum(span.latency_us for span in self.local_spans)
        child = sum(child.total_latency_us() for child in self.children)
        return local + child

    def local_latency_us(self) -> int:
        """Return the latency attributable to spans on this node only."""
        return sum(span.latency_us for span in self.local_spans)

    def get_all_descendants(self) -> List["ServiceNode"]:
        """Recursively collects all children and their descendants into a flat list."""
        descendants = []
        for child in self.children:
            descendants.append(child)
            descendants.extend(child.get_all_descendants())
        return descendants

    def get_local_spans_as_trace(self) -> List[dict]:
        """Renders only the local spans of this node for trace output."""
        return [span.to_trace_span() for span in self.local_spans]

    def to_trace_spans_original(self) -> List[dict]:
        """Generates spans with the original logic (children appended at the end)."""
        spans: List[dict] = [span.to_trace_span() for span in self.local_spans]
        for child in self.children:
            spans.append(
                {
                    "type": "ChildCall",
                    "service_name": child.name,
                    "latency_us": child.local_latency_us(),
                    "start_timestamp": child.start_timestamp,
                    "spans": child.to_trace_spans_original(),  # Recursive call
                }
            )
        return spans


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Convert trace CSV to JSON artefacts")
    parser.add_argument("--input", required=True, help="Path to the raw CSV trace file")
    parser.add_argument(
        "--output-dir",
        required=True,
        help="Destination directory for the generated JSON files",
    )
    parser.add_argument(
        "--prefix",
        help=(
            "Filename prefix for generated files; defaults to the input stem (without extension)"
        ),
    )
    return parser.parse_args()


def split_tokens(tokens: Sequence[str]) -> Tuple[List[str], List[str]]:
    """Split tokens into (local span tokens, remaining tokens)."""
    for idx, token in enumerate(tokens):
        if not SPAN_PATTERN.fullmatch(token):
            return list(tokens[:idx]), list(tokens[idx:])
    return list(tokens), []


def parse_local_spans(tokens: Sequence[str]) -> List[LocalSpan]:
    spans: List[LocalSpan] = []
    for token in tokens:
        match = SPAN_PATTERN.fullmatch(token)
        if not match:
            raise ValueError(f"Unexpected token in local span list: {token!r}")
        kind = match.group("kind")
        value = int(match.group("value"))
        if kind == "Queueing":
            continue
        spans.append(LocalSpan(raw_kind=kind, latency_us=value))
    return spans


def parse_service(tokens: Sequence[str], index: int) -> Tuple[ServiceNode, int]:
    if index >= len(tokens):
        raise ValueError("parse_service called with index beyond token list")

    name = tokens[index]
    if SPAN_PATTERN.fullmatch(name):
        raise ValueError(f"Expected service name, found span token {name!r}")

    if index + 1 >= len(tokens):
        raise ValueError(f"Missing start timestamp for child service {name!r}")

    try:
        start_ts = int(tokens[index + 1])
    except ValueError as exc:
        raise ValueError(f"Invalid start timestamp for service {name!r}") from exc

    local_spans: List[LocalSpan] = []
    children: List[ServiceNode] = []

    idx = index + 2
    while idx < len(tokens):
        token = tokens[idx]
        match = SPAN_PATTERN.fullmatch(token)
        if not match:
            break
        span_kind = match.group("kind")
        if span_kind == "Queueing":
            idx += 1
            continue
        latency_us = int(match.group("value"))
        local_spans.append(LocalSpan(raw_kind=span_kind, latency_us=latency_us))
        idx += 1

    while idx < len(tokens):
        token = tokens[idx]
        if SPAN_PATTERN.fullmatch(token):
            break
        child, idx = parse_service(tokens, idx)
        children.append(child)

    node = ServiceNode(name=name, start_timestamp=start_ts, local_spans=local_spans, children=children)
    return node, idx


def parse_child_nodes(tokens: Sequence[str]) -> List[ServiceNode]:
    children: List[ServiceNode] = []
    idx = 0
    while idx < len(tokens):
        token = tokens[idx]
        if SPAN_PATTERN.fullmatch(token):
            raise ValueError(
                f"Unexpected span token {token!r} while parsing child services"
            )
        child, idx = parse_service(tokens, idx)
        children.append(child)
    return children


def prepare_output_path(args: argparse.Namespace) -> Path:
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    prefix = args.prefix if args.prefix else Path(args.input).stem
    return output_dir / f"{prefix}_frontend_modified.json"


def process_trace_file(input_path: Path) -> List[dict]:
    frontend_requests_modified: List[dict] = []

    with input_path.open(newline="") as f:
        reader = csv.reader(f)
        for row in reader:
            if not row or row[0].strip().lower() == "api":
                continue
            if len(row) < 7:
                raise ValueError(f"Row has insufficient columns: {row}")

            api, request_id, _, start_at, _, latency, *_ = row
            request_id, start_at, latency = int(request_id), int(start_at), int(latency)
            tail_tokens = [token.strip() for token in row[7:] if token.strip()]

            root_node = ServiceNode(name=api.strip(), start_timestamp=start_at)
            frontend_tokens, remainder_tokens = split_tokens(tail_tokens)
            root_node.local_spans = parse_local_spans(frontend_tokens)
            root_node.children = parse_child_nodes(remainder_tokens)

            # --- Generate MODIFIED output ---
            original_spans = root_node.to_trace_spans_original()
            modified_spans: List[dict] = []
            has_child_block_at_root = any(s.raw_kind == "ChildBlock" for s in root_node.local_spans)

            if has_child_block_at_root:
                all_descendants = root_node.get_all_descendants()
                descendant_iterator = iter(all_descendants)
                for span in root_node.local_spans:
                    if span.raw_kind == "ChildBlock":
                        try:
                            child_node = next(descendant_iterator)
                            modified_spans.append({
                                "type": "ChildCall",
                                "service_name": child_node.name,
                                "latency_us": child_node.local_latency_us(),
                                "start_timestamp": child_node.start_timestamp,
                                "spans": child_node.get_local_spans_as_trace(),
                            })
                        except StopIteration:
                            modified_spans.append({"type": "Block", "latency_us": span.latency_us})
                    else:
                        modified_spans.append(span.to_trace_span())
            else:
                modified_spans = original_spans

            modified_span_latency = sum_span_latency(modified_spans)
            frontend_requests_modified.append({
                "service_name": root_node.name,
                "request_id": request_id,
                "start_at": start_at,
                "traced_latency_us": latency,
                "span_latency_us": modified_span_latency,
                "spans": modified_spans,
            })

    return frontend_requests_modified


def main() -> None:
    args = parse_args()
    input_path = Path(args.input)
    modified_path = prepare_output_path(args)

    frontend_modified = process_trace_file(input_path)

    modified_path.write_text(json.dumps(frontend_modified, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
