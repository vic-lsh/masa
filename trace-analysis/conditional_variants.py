#!/usr/bin/env python
"""Build conditional call-sequence variants from Alibaba MSCallGraph traces.

This script reconstructs each trace from rpc_id parentage and span timing,
extracts each RPC node's local ordered fanout sequence, and learns conditional
variant mappings:

    parent method variant -> child call slot -> child method variant distribution

The output is an analysis artifact for improving MSSim call-sequence replay. It
does not change the existing call_sequence.json format.
"""

from __future__ import annotations

import argparse
import json
import logging
import re
import sys
from collections import Counter, defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import pandas as pd

logger = logging.getLogger(__name__)

TRACE_COLUMNS = [
    "timestamp",
    "traceid",
    "service",
    "rpc_id",
    "rpctype",
    "um",
    "interface",
    "dm",
    "rt",
]

Signature = tuple[tuple[str, ...], ...]
TransitionKey = tuple[str, Signature, int, str, int]


@dataclass
class Span:
    rpc_id: str
    method: str
    dm: str
    interface: str
    start: float
    end: float
    parent_rpc_id: str | None


def project_root() -> Path:
    cwd = Path.cwd()
    if (cwd / "traces").exists():
        return cwd
    return Path(__file__).resolve().parent.parent


def default_trace_dir() -> Path:
    return (
        project_root()
        / "traces"
        / "alibaba"
        / "cluster-trace-microservices-v2022"
        / "data"
        / "CallGraph"
    )


def normalize_interface(value: object) -> str:
    if value is None or pd.isna(value):
        return "<none>"
    text = str(value).strip()
    if not text or text.lower() == "nan":
        return "<none>"
    return text


def format_method(dm: str, interface: object) -> str:
    return f"{str(dm).strip()}::{normalize_interface(interface)}"


def parent_rpc_id(rpc_id: object) -> str | None:
    if rpc_id is None:
        return None
    try:
        if isinstance(rpc_id, float) and pd.isna(rpc_id):
            return None
    except (TypeError, ValueError):
        pass

    rpc = str(rpc_id).strip()
    if not rpc or rpc.lower() == "nan" or "." not in rpc:
        return None
    return rpc.rsplit(".", 1)[0]


def slugify(value: str) -> str:
    text = re.sub(r"[^\w\-]+", "_", value.strip())
    text = re.sub(r"_+", "_", text).strip("_")
    return text or "output"


def mode_text(values: pd.Series, default: str = "<none>") -> str:
    normalized = [
        str(value).strip()
        for value in values.dropna().tolist()
        if str(value).strip() and str(value).strip().lower() != "nan"
    ]
    if not normalized:
        return default
    counts = Counter(normalized)
    return sorted(counts.items(), key=lambda item: (-item[1], item[0]))[0][0]


def parse_dataset_ids(raw_ids: str | None, num_datasets: int) -> list[int]:
    if raw_ids:
        ids: list[int] = []
        for part in raw_ids.split(","):
            part = part.strip()
            if not part:
                continue
            if "-" in part:
                start, end = part.split("-", 1)
                ids.extend(range(int(start), int(end) + 1))
            else:
                ids.append(int(part))
        return sorted(set(ids))
    return list(range(num_datasets))


def read_callgraph_csvs(
    trace_dir: Path,
    dataset_ids: list[int],
    max_rows: int | None,
) -> pd.DataFrame:
    frames: list[pd.DataFrame] = []
    for dataset_id in dataset_ids:
        path = trace_dir / f"CallGraph_{dataset_id}.csv"
        if not path.exists():
            raise FileNotFoundError(path)
        logger.info("Reading %s", path)
        frame = pd.read_csv(
            path,
            usecols=lambda col: col in TRACE_COLUMNS,
            nrows=max_rows,
            on_bad_lines="skip",
        )
        frame["_dataset_id"] = str(dataset_id)
        frames.append(frame)
    if not frames:
        return pd.DataFrame(columns=TRACE_COLUMNS + ["_dataset_id", "_trace_key"])

    df = pd.concat(frames, ignore_index=True, sort=False)
    missing = [col for col in TRACE_COLUMNS if col not in df.columns]
    if missing:
        raise ValueError(f"Missing required columns in input CSVs: {missing}")

    df["traceid"] = df["traceid"].astype(str).str.strip()
    df["service"] = df["service"].astype(str).str.strip()
    df["rpc_id"] = df["rpc_id"].astype(str).str.strip()
    df["um"] = df["um"].astype(str).str.strip()
    df["dm"] = df["dm"].astype(str).str.strip()
    df["_trace_key"] = df["_dataset_id"] + ":" + df["traceid"]
    df["timestamp"] = pd.to_numeric(df["timestamp"], errors="coerce")
    df["rt"] = pd.to_numeric(df["rt"], errors="coerce")
    return df


def clean_data(df: pd.DataFrame, excluded_rpctypes: set[str]) -> pd.DataFrame:
    original_len = len(df)
    df = df[
        ~df["um"].isin(["UNKNOWN", "UNAVAILABLE"])
        & ~df["dm"].isin(["UNKNOWN", "UNAVAILABLE"])
    ].copy()
    if excluded_rpctypes:
        df = df[~df["rpctype"].astype(str).str.strip().isin(excluded_rpctypes)].copy()
    logger.info("Filtered rows from %s to %s", f"{original_len:,}", f"{len(df):,}")
    return df


def sample_trace_keys(
    df: pd.DataFrame,
    fraction: float,
    random_state: int,
) -> pd.DataFrame:
    if fraction >= 1.0:
        return df
    trace_keys = pd.Series(df["_trace_key"].dropna().unique())
    if trace_keys.empty:
        return df.iloc[0:0].copy()
    keep_count = max(1, int(len(trace_keys) * fraction))
    keep = set(trace_keys.sample(n=keep_count, random_state=random_state).tolist())
    sampled = df[df["_trace_key"].isin(keep)].copy()
    logger.info(
        "Sampled %s of %s trace keys; kept %s rows",
        f"{keep_count:,}",
        f"{len(trace_keys):,}",
        f"{len(sampled):,}",
    )
    return sampled


def build_spans(trace_df: pd.DataFrame) -> tuple[dict[str, Span], Counter[str]]:
    spans: dict[str, Span] = {}
    stats: Counter[str] = Counter()

    for rpc_id, group in trace_df.groupby("rpc_id", sort=False):
        rpc = str(rpc_id).strip()
        if not rpc or rpc.lower() == "nan":
            stats["invalid_rpc_id"] += len(group)
            continue

        dm = mode_text(group["dm"], default="")
        if not dm:
            stats["missing_dm"] += len(group)
            continue

        interface = normalize_interface(mode_text(group["interface"], default="<none>"))
        valid_starts = group["timestamp"].dropna()
        valid_ends = (group["timestamp"] + group["rt"]).dropna()
        if valid_starts.empty or valid_ends.empty:
            stats["missing_timing"] += len(group)
            continue

        start = float(valid_starts.min())
        end = float(valid_ends.max())
        if end < start:
            stats["negative_duration"] += len(group)
            continue

        spans[rpc] = Span(
            rpc_id=rpc,
            method=format_method(dm, interface),
            dm=dm,
            interface=interface,
            start=start,
            end=end,
            parent_rpc_id=parent_rpc_id(rpc),
        )

    return spans, stats


def clamp_child_spans(
    spans: dict[str, Span],
    children_by_parent: dict[str, list[Span]],
) -> int:
    clamped = 0
    for parent_id, children in children_by_parent.items():
        parent = spans.get(parent_id)
        if parent is None:
            continue
        for child in children:
            if child.start < parent.start or child.end > parent.end:
                child.start = max(child.start, parent.start)
                child.end = min(child.end, parent.end)
                if child.end <= child.start:
                    child.end = child.start + 1.0
                clamped += 1
    return clamped


def spans_overlap(a: Span, b: Span, overlap_threshold: float) -> bool:
    overlap_start = max(a.start, b.start)
    overlap_end = min(a.end, b.end)
    if overlap_start >= overlap_end:
        return False

    min_duration = min(a.end - a.start, b.end - b.start)
    if min_duration <= 0:
        return False
    return ((overlap_end - overlap_start) / min_duration) >= overlap_threshold


def group_fanout(children: list[Span], overlap_threshold: float) -> list[list[Span]]:
    if not children:
        return []

    sorted_children = sorted(children, key=lambda span: (span.start, span.end, span.method, span.rpc_id))
    groups: list[list[Span]] = []
    current = [sorted_children[0]]

    for child in sorted_children[1:]:
        if any(spans_overlap(child, existing, overlap_threshold) for existing in current):
            current.append(child)
        else:
            groups.append(sorted(current, key=lambda span: (span.method, span.rpc_id)))
            current = [child]

    groups.append(sorted(current, key=lambda span: (span.method, span.rpc_id)))
    return groups


def signature_from_groups(groups: list[list[Span]]) -> Signature:
    return tuple(tuple(child.method for child in group) for group in groups)


def add_transition_counts(
    parent_method: str,
    parent_signature: Signature,
    groups: list[list[Span]],
    signature_by_rpc: dict[str, Signature],
    transition_counts: dict[TransitionKey, Counter[Signature]],
) -> None:
    for group_idx, group in enumerate(groups):
        duplicate_index: Counter[str] = Counter()
        for child in group:
            occurrence_idx = duplicate_index[child.method]
            duplicate_index[child.method] += 1
            child_signature = signature_by_rpc.get(child.rpc_id)
            if child_signature is None:
                continue
            key = (
                parent_method,
                parent_signature,
                group_idx,
                child.method,
                occurrence_idx,
            )
            transition_counts[key][child_signature] += 1


def analyze_service(
    service_name: str,
    service_df: pd.DataFrame,
    overlap_threshold: float,
    missing_parent_as_root: bool,
) -> tuple[dict[str, Any], dict[str, Any]]:
    method_signature_counts: dict[str, Counter[Signature]] = defaultdict(Counter)
    method_occurrence_counts: Counter[str] = Counter()
    transition_counts: dict[TransitionKey, Counter[Signature]] = defaultdict(Counter)
    stats: Counter[str] = Counter()

    trace_groups = service_df.groupby("_trace_key", sort=False)
    stats["trace_count"] = len(trace_groups)
    stats["row_count"] = len(service_df)

    for _trace_key, trace_df in trace_groups:
        spans, span_stats = build_spans(trace_df)
        stats.update(span_stats)
        if not spans:
            stats["empty_traces"] += 1
            continue

        stats["span_count"] += len(spans)
        children_by_parent: dict[str, list[Span]] = defaultdict(list)
        root_spans: list[Span] = []

        for span in spans.values():
            parent_id = span.parent_rpc_id
            if parent_id is None:
                root_spans.append(span)
            elif parent_id in spans:
                children_by_parent[parent_id].append(span)
            elif missing_parent_as_root:
                root_spans.append(span)
                stats["missing_parent_root_spans"] += 1
            else:
                stats["missing_parent_spans"] += 1

        stats["root_span_count"] += len(root_spans)
        stats["clamped_spans"] += clamp_child_spans(spans, children_by_parent)

        local_groups_by_rpc: dict[str, list[list[Span]]] = {}
        signature_by_rpc: dict[str, Signature] = {}

        for rpc_id, span in spans.items():
            groups = group_fanout(children_by_parent.get(rpc_id, []), overlap_threshold)
            signature = signature_from_groups(groups)
            local_groups_by_rpc[rpc_id] = groups
            signature_by_rpc[rpc_id] = signature
            method_signature_counts[span.method][signature] += 1
            method_occurrence_counts[span.method] += 1

        if root_spans:
            root_groups = group_fanout(root_spans, overlap_threshold)
            root_signature = signature_from_groups(root_groups)
            method_signature_counts["USER"][root_signature] += 1
            method_occurrence_counts["USER"] += 1
            add_transition_counts(
                "USER",
                root_signature,
                root_groups,
                signature_by_rpc,
                transition_counts,
            )

        for rpc_id, groups in local_groups_by_rpc.items():
            span = spans[rpc_id]
            parent_signature = signature_by_rpc[rpc_id]
            add_transition_counts(
                span.method,
                parent_signature,
                groups,
                signature_by_rpc,
                transition_counts,
            )

    variant_ids = assign_variant_ids(method_signature_counts)
    methods = build_methods_payload(
        method_signature_counts,
        method_occurrence_counts,
        transition_counts,
        variant_ids,
    )

    graph_payload = {
        "trace_count": int(stats["trace_count"]),
        "row_count": int(stats["row_count"]),
        "span_count": int(stats["span_count"]),
        "root_span_count": int(stats["root_span_count"]),
        "method_count": len(methods),
        "variant_count": sum(len(method["variants"]) for method in methods.values()),
        "methods": methods,
    }
    return graph_payload, {key: int(value) for key, value in sorted(stats.items())}


def assign_variant_ids(
    method_signature_counts: dict[str, Counter[Signature]],
) -> dict[str, dict[Signature, str]]:
    variant_ids: dict[str, dict[Signature, str]] = {}
    for method, counter in method_signature_counts.items():
        ordered = sorted(counter.items(), key=lambda item: (-item[1], item[0]))
        variant_ids[method] = {
            signature: f"v{idx:03d}" for idx, (signature, _count) in enumerate(ordered, 1)
        }
    return variant_ids


def variant_distribution_payload(
    child_method: str,
    counter: Counter[Signature],
    variant_ids: dict[str, dict[Signature, str]],
) -> tuple[dict[str, int], dict[str, float]]:
    total = sum(counter.values())
    counts: dict[str, int] = {}
    probabilities: dict[str, float] = {}
    if total <= 0:
        return counts, probabilities

    for signature, count in sorted(counter.items(), key=lambda item: (-item[1], item[0])):
        variant_id = variant_ids.get(child_method, {}).get(signature)
        if variant_id is None:
            variant_id = "unknown"
        counts[variant_id] = int(count)
        probabilities[variant_id] = count / total
    return counts, probabilities


def build_methods_payload(
    method_signature_counts: dict[str, Counter[Signature]],
    method_occurrence_counts: Counter[str],
    transition_counts: dict[TransitionKey, Counter[Signature]],
    variant_ids: dict[str, dict[Signature, str]],
) -> dict[str, Any]:
    methods: dict[str, Any] = {}
    ordered_methods = sorted(method_signature_counts.keys(), key=lambda name: (name != "USER", name))

    for method in ordered_methods:
        total = method_occurrence_counts[method]
        variants: dict[str, Any] = {}
        ordered_signatures = sorted(
            method_signature_counts[method].items(),
            key=lambda item: variant_ids[method][item[0]],
        )

        for signature, count in ordered_signatures:
            sequence: list[list[dict[str, Any]]] = []
            for group_idx, group in enumerate(signature):
                duplicate_index: Counter[str] = Counter()
                output_group: list[dict[str, Any]] = []
                for target in group:
                    occurrence_idx = duplicate_index[target]
                    duplicate_index[target] += 1
                    key = (method, signature, group_idx, target, occurrence_idx)
                    child_counter = transition_counts.get(key, Counter())

                    entry: dict[str, Any] = {
                        "target": target,
                        "slot": {"group": group_idx, "index": len(output_group)},
                    }
                    if child_counter:
                        counts, probabilities = variant_distribution_payload(
                            target,
                            child_counter,
                            variant_ids,
                        )
                        entry["callee_variant_counts"] = counts
                        entry["callee_variants"] = probabilities
                    output_group.append(entry)
                sequence.append(output_group)

            variant_id = variant_ids[method][signature]
            variants[variant_id] = {
                "count": int(count),
                "probability": count / total if total else 0.0,
                "sequence": sequence,
            }

        methods[method] = {
            "count": int(total),
            "variants": variants,
        }

    return methods


def select_services(
    df: pd.DataFrame,
    requested_services: list[str],
    top_services: int,
) -> list[str]:
    if requested_services:
        available = set(df["service"].dropna().unique())
        missing = [service for service in requested_services if service not in available]
        if missing:
            logger.warning("Requested services not present in sample: %s", ", ".join(missing))
        return [service for service in requested_services if service in available]

    counts = (
        df.groupby("service")["_trace_key"]
        .nunique()
        .sort_values(ascending=False)
    )
    return counts.head(top_services).index.tolist()


def write_outputs(
    output_dir: Path,
    payload: dict[str, Any],
    summary_rows: list[dict[str, Any]],
) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    variants_path = output_dir / "conditional_variants.json"
    summary_path = output_dir / "summary.csv"
    variants_path.write_text(json.dumps(payload, indent=2, sort_keys=True), encoding="utf-8")
    pd.DataFrame(summary_rows).to_csv(summary_path, index=False)
    logger.info("Wrote %s", variants_path)
    logger.info("Wrote %s", summary_path)


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Extract conditional fanout sequence variants from Alibaba MSCallGraph CSVs."
    )
    parser.add_argument("--trace-dir", type=Path, default=default_trace_dir())
    parser.add_argument(
        "--num-datasets",
        type=int,
        default=1,
        help="Number of CallGraph_N.csv files to read, starting at 0.",
    )
    parser.add_argument(
        "--dataset-ids",
        default=None,
        help="Comma/range list such as '0,2,4-6'. Overrides --num-datasets.",
    )
    parser.add_argument(
        "--max-rows",
        type=int,
        default=None,
        help="Maximum rows to read per CSV. This is for quick samples and may truncate traces.",
    )
    parser.add_argument(
        "--sample-trace-frac",
        type=float,
        default=1.0,
        help="Sample this fraction of loaded trace IDs after row loading.",
    )
    parser.add_argument("--random-state", type=int, default=42)
    parser.add_argument(
        "--top-services",
        type=int,
        default=5,
        help="Analyze the top N online services by trace count unless --service is provided.",
    )
    parser.add_argument(
        "--service",
        action="append",
        default=[],
        help="Analyze one service ID. Can be repeated.",
    )
    parser.add_argument(
        "--overlap-threshold",
        type=float,
        default=0.1,
        help="Minimum fraction of shorter span overlap to group siblings into one fanout stage.",
    )
    parser.add_argument(
        "--exclude-rpctype",
        default="UNKNOWN,mq",
        help="Comma-separated rpctype values to exclude. Use an empty string to keep all.",
    )
    parser.add_argument(
        "--missing-parent-as-root",
        action=argparse.BooleanOptionalAction,
        default=True,
        help=(
            "Treat spans whose rpc_id parent is absent from the trace as top-level "
            "observed roots. This handles Alibaba traces whose first visible rpc_id "
            "is often 0.1 rather than 0."
        ),
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path(__file__).resolve().parent / "conditional_variants",
    )
    return parser


def main() -> None:
    logging.basicConfig(
        level=logging.INFO,
        format="%(asctime)s - %(levelname)s - %(message)s",
        stream=sys.stderr,
    )
    parser = build_arg_parser()
    args = parser.parse_args()

    if args.num_datasets < 1:
        parser.error("--num-datasets must be at least 1")
    if not (0.0 < args.sample_trace_frac <= 1.0):
        parser.error("--sample-trace-frac must be in (0.0, 1.0]")
    if not (0.0 <= args.overlap_threshold <= 1.0):
        parser.error("--overlap-threshold must be in [0.0, 1.0]")

    dataset_ids = parse_dataset_ids(args.dataset_ids, args.num_datasets)
    excluded = {item.strip() for item in args.exclude_rpctype.split(",") if item.strip()}

    df = read_callgraph_csvs(args.trace_dir, dataset_ids, args.max_rows)
    df = clean_data(df, excluded)
    df = sample_trace_keys(df, args.sample_trace_frac, args.random_state)

    services = select_services(df, args.service, args.top_services)
    if not services:
        raise SystemExit("No services selected for analysis")
    logger.info("Analyzing services: %s", ", ".join(services))

    payload: dict[str, Any] = {
        "schema_version": 1,
        "metadata": {
            "trace_dir": str(args.trace_dir),
            "dataset_ids": dataset_ids,
            "max_rows_per_csv": args.max_rows,
            "sample_trace_frac": args.sample_trace_frac,
            "random_state": args.random_state,
            "overlap_threshold": args.overlap_threshold,
            "excluded_rpctypes": sorted(excluded),
            "selected_services": services,
            "missing_parent_as_root": args.missing_parent_as_root,
        },
        "graphs": {},
    }
    summary_rows: list[dict[str, Any]] = []

    for service_name in services:
        service_df = df[df["service"] == service_name].copy()
        logger.info(
            "Processing %s (%s rows, %s traces)",
            service_name,
            f"{len(service_df):,}",
            f"{service_df['_trace_key'].nunique():,}",
        )
        graph_payload, stats = analyze_service(
            service_name,
            service_df,
            args.overlap_threshold,
            args.missing_parent_as_root,
        )
        payload["graphs"][service_name] = graph_payload

        summary_row = {
            "service": service_name,
            "trace_count": graph_payload["trace_count"],
            "row_count": graph_payload["row_count"],
            "span_count": graph_payload["span_count"],
            "root_span_count": graph_payload["root_span_count"],
            "method_count": graph_payload["method_count"],
            "variant_count": graph_payload["variant_count"],
        }
        summary_row.update({f"stat_{key}": value for key, value in stats.items()})
        summary_rows.append(summary_row)

    write_outputs(args.output_dir, payload, summary_rows)


if __name__ == "__main__":
    main()
