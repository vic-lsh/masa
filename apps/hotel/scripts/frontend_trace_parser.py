#!/usr/bin/env python3
"""Parse trace CSV files into JSON suitable for load replay."""

from __future__ import annotations

import argparse
import csv
import json
import re
from dataclasses import dataclass, field
from pathlib import Path
from typing import List, Sequence, Tuple

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

    def to_dict(self) -> dict:
        return {"kind": self.raw_kind, "latency_us": self.latency_us}


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

    def to_trace_spans(self) -> List[dict]:
        spans: List[dict] = [span.to_trace_span() for span in self.local_spans]
        for child in self.children:
            spans.append(
                {
                    "type": "ChildCall",
                    "service_name": child.name,
                    "latency_us": child.total_latency_us(),
                    "start_timestamp": child.start_timestamp,
                    "spans": child.to_trace_spans(),
                }
            )
        return spans

    def to_dict(self) -> dict:
        return {
            "service_name": self.name,
            "start_timestamp": self.start_timestamp,
            "latency_us": self.total_latency_us(),
            "spans": [span.to_dict() for span in self.local_spans],
            "children": [child.to_dict() for child in self.children],
        }


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
            # Encountering a span means we've reached the parent's continuation.
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


def prepare_output_paths(args: argparse.Namespace) -> Tuple[Path, Path, Path]:
    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    prefix = args.prefix if args.prefix else Path(args.input).stem
    basic_path = output_dir / f"{prefix}_basic.json"
    frontend_path = output_dir / f"{prefix}_frontend.json"
    children_path = output_dir / f"{prefix}_children.json"
    return basic_path, frontend_path, children_path


def process_trace_file(input_path: Path) -> Tuple[List[dict], List[dict], List[dict]]:
    basic_records: List[dict] = []
    frontend_requests: List[dict] = []
    child_records: List[dict] = []

    with input_path.open(newline="") as f:
        reader = csv.reader(f)
        for row in reader:
            if not row:
                continue

            # Handle header row if present
            header_candidate = row[0].strip().lower()
            if header_candidate == "api":
                continue

            if len(row) < 7:
                raise ValueError(f"Row has insufficient columns: {row}")

            api = row[0].strip()
            request_id = int(row[1])
            slo = int(row[2])
            start_at = int(row[3])
            deadline = int(row[4])
            latency = int(row[5])
            raw_error = row[6].strip()
            error = None if raw_error in {"/None", "None", ""} else raw_error

            tail_tokens = [token.strip() for token in row[7:] if token.strip()]
            frontend_tokens, remainder_tokens = split_tokens(tail_tokens)

            frontend_spans = parse_local_spans(frontend_tokens)
            child_nodes = parse_child_nodes(remainder_tokens) if remainder_tokens else []

            basic_records.append(
                {
                    "api": api,
                    "request_id": request_id,
                    "slo_us": slo,
                    "start_at": start_at,
                    "deadline": deadline,
                    "latency_us": latency,
                    "error": error,
                }
            )

            node_repr = [child.to_dict() for child in child_nodes]
            child_records.append(
                {
                    "api": api,
                    "request_id": request_id,
                    "children": node_repr,
                }
            )

            frontend_trace_spans = [span.to_trace_span() for span in frontend_spans]
            for child in child_nodes:
                frontend_trace_spans.append(
                    {
                        "type": "ChildCall",
                        "service_name": child.name,
                        "latency_us": child.total_latency_us(),
                        "start_timestamp": child.start_timestamp,
                        "spans": child.to_trace_spans(),
                    }
                )

            frontend_requests.append(
                {
                    "service_name": api,
                    "request_id": request_id,
                    "start_at": start_at,
                    "latency_us": latency,
                    "spans": frontend_trace_spans,
                }
            )

    return basic_records, frontend_requests, child_records


def frontend_local_only_json(frontend_requests: List[dict]) -> str:
    """Return frontend replay payload with child calls collapsed to local blocks."""

    def rewrite_spans(spans: List[dict]) -> List[dict]:
        collapsed: List[dict] = []
        for span in spans:
            span_type = span.get("type")
            if span_type == "ChildCall":
                collapsed.append({
                    "type": "Block",
                    "latency_us": span.get("latency_us", 0),
                })
            else:
                collapsed.append(dict(span))
        return collapsed

    rewritten: List[dict] = []
    for request in frontend_requests:
        updated = dict(request)
        updated["spans"] = rewrite_spans(request.get("spans", []))
        rewritten.append(updated)

    return json.dumps(rewritten, indent=2)


def main() -> None:
    args = parse_args()
    input_path = Path(args.input)
    basic_path, frontend_path, children_path = prepare_output_paths(args)

    basic, frontend, children = process_trace_file(input_path)

    for path, payload in [
        (basic_path, basic),
        (frontend_path, frontend),
        (children_path, children),
    ]:
        path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
