#!/usr/bin/env python3
"""Analyze the controlled estimator-ordering experiments.

The analyzer intentionally treats estimator telemetry as optional.  End-to-end,
class, queueing, and realized-ordering metrics are still emitted for policies
that do not contain ``EST_AUDIT_JSON`` records.  Audit-dependent fields are
``null`` instead of silently substituting an API-level estimate.
"""

from __future__ import annotations

import argparse
import csv
import itertools
import json
import math
import re
import statistics
from collections import defaultdict
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Iterable, Iterator, Mapping, Sequence


AUDIT_PREFIX = "EST_AUDIT_JSON:"
RESULT_FILE = re.compile(r"r(?P<rps>\d+)_(?P<api>.+)\.csv")
AMBIGUITY_THRESHOLDS = (0.01, 0.05, 0.10, 0.25)
NO_ERROR = {"", "none", "/none", "null", "nan"}


@dataclass(frozen=True)
class RequestResult:
    api: str
    request_id: int
    slo_us: float
    gateway_entry_us: int
    deadline_us: int
    latency_us: float
    succeeded: bool
    excluded_from_goodput: bool
    request_class: str
    shared_started_at_us: int | None
    shared_finished_at_us: int | None
    shared_queue_us: float | None
    plan_reference_us: float | None


def _number(value: Any) -> float | None:
    if value is None or isinstance(value, bool):
        return None
    try:
        result = float(value)
    except (TypeError, ValueError):
        return None
    return result if math.isfinite(result) else None


def _integer(value: Any) -> int | None:
    number = _number(value)
    return None if number is None else int(number)


def percentile(values: Iterable[float], q: float) -> float | None:
    """Return the type-7 sample percentile used by common plotting tools."""

    ordered = sorted(value for value in values if math.isfinite(value))
    if not ordered:
        return None
    if len(ordered) == 1:
        return ordered[0]
    position = (len(ordered) - 1) * q
    lower = math.floor(position)
    upper = math.ceil(position)
    if lower == upper:
        return ordered[lower]
    weight = position - lower
    return ordered[lower] * (1.0 - weight) + ordered[upper] * weight


def safe_fraction(numerator: float, denominator: float) -> float | None:
    return None if denominator == 0 else numerator / denominator


def _cmp(left: float, right: float) -> int:
    return (left > right) - (left < right)


def _walk_hops(hops: Sequence[Mapping[str, Any]]) -> Iterator[Mapping[str, Any]]:
    for hop in hops:
        yield hop
        children = hop.get("children", [])
        if isinstance(children, list):
            yield from _walk_hops(children)


def _parse_hops(raw: str) -> list[Mapping[str, Any]]:
    try:
        value = json.loads(raw or "[]")
    except json.JSONDecodeError:
        return []
    return value if isinstance(value, list) else []


def _suffix_reference(hops: Sequence[Mapping[str, Any]]) -> float | None:
    """Configured work after the shared hop for the linear controlled sweep."""

    shared_index = next(
        (
            index
            for index, hop in enumerate(hops)
            if str(hop.get("service_id", "")) == "shared"
        ),
        None,
    )
    if shared_index is None:
        return None
    values = [
        _number(hop.get("configured_work_us")) for hop in hops[shared_index + 1 :]
    ]
    return sum(value for value in values if value is not None)


def _load_request_results(policy_dir: Path, rps: int) -> list[RequestResult]:
    results: list[RequestResult] = []
    for result_path in sorted(policy_dir.glob(f"r{rps}_*.csv")):
        match = RESULT_FILE.fullmatch(result_path.name)
        if match is None:
            continue
        file_api = match.group("api")
        with result_path.open(newline="", encoding="utf-8") as result_file:
            for row in csv.DictReader(result_file):
                request_id = _integer(row.get("request_id"))
                slo_us = _number(row.get("slo"))
                gateway_entry_us = _integer(row.get("start_at"))
                deadline_us = _integer(row.get("deadline"))
                latency_us = _number(row.get("latency"))
                if None in (
                    request_id,
                    slo_us,
                    gateway_entry_us,
                    deadline_us,
                    latency_us,
                ):
                    continue

                hops = _parse_hops(row.get("hop_trace_json", ""))
                flat_hops = list(_walk_hops(hops))
                shared = next(
                    (
                        hop
                        for hop in flat_hops
                        if str(hop.get("service_id", "")) == "shared"
                    ),
                    None,
                )
                is_long = any(
                    str(hop.get("service_id", "")) == "long-extra" for hop in flat_hops
                )
                error = str(row.get("error", "")).strip().lower()
                excluded = error == "/clienttimeout" or error.startswith("/earlyreturn")
                results.append(
                    RequestResult(
                        api=str(row.get("api") or file_api),
                        request_id=request_id,
                        slo_us=slo_us,
                        gateway_entry_us=gateway_entry_us,
                        deadline_us=deadline_us,
                        latency_us=latency_us,
                        succeeded=error in NO_ERROR,
                        excluded_from_goodput=excluded,
                        request_class="long" if is_long else "short",
                        shared_started_at_us=(
                            _integer(shared.get("started_at")) if shared else None
                        ),
                        shared_finished_at_us=(
                            _integer(shared.get("finished_at")) if shared else None
                        ),
                        shared_queue_us=(
                            _number(shared.get("queueing_latency_us"))
                            if shared
                            else None
                        ),
                        plan_reference_us=_suffix_reference(hops),
                    )
                )
    return results


def parse_audit_logs(policy_dir: Path) -> list[dict[str, Any]]:
    """Parse audit JSON, ignoring unrelated and malformed service log lines."""

    events: list[dict[str, Any]] = []
    logs_dir = policy_dir / "logs"
    if not logs_dir.exists():
        return events
    for log_path in sorted(logs_dir.rglob("*.log")):
        with log_path.open(encoding="utf-8", errors="replace") as log_file:
            for line_number, line in enumerate(log_file, 1):
                marker = line.find(AUDIT_PREFIX)
                if marker < 0:
                    continue
                raw = line[marker + len(AUDIT_PREFIX) :].strip()
                try:
                    event = json.loads(raw)
                except json.JSONDecodeError:
                    continue
                if not isinstance(event, dict):
                    continue
                event = dict(event)
                event["_container_log"] = log_path.name
                event["_line"] = line_number
                events.append(event)
    return events


def _event_field(event: Mapping[str, Any], *names: str) -> Any:
    for name in names:
        if name in event:
            return event[name]
    return None


def _is_shared_lookup(event: Mapping[str, Any]) -> bool:
    child = str(_event_field(event, "child_method", "child", "target_method") or "")
    return child == "shared::run" or child.endswith("::shared::run")


def _audit_index(
    events: Sequence[Mapping[str, Any]],
) -> dict[tuple[int, int | None], list[Mapping[str, Any]]]:
    index: dict[tuple[int, int | None], list[Mapping[str, Any]]] = defaultdict(list)
    for event in events:
        if not _is_shared_lookup(event):
            continue
        request_id = _integer(_event_field(event, "request_id"))
        if request_id is None:
            continue
        gateway = _integer(
            _event_field(event, "gateway_entry_us", "gateway_entry", "start_at")
        )
        index[(request_id, gateway)].append(event)
        if gateway is not None:
            index[(request_id, None)].append(event)
    return index


def _choose_audit_event(
    index: Mapping[tuple[int, int | None], Sequence[Mapping[str, Any]]],
    request: RequestResult,
) -> Mapping[str, Any] | None:
    exact = index.get((request.request_id, request.gateway_entry_us), ())
    candidates = exact or index.get((request.request_id, None), ())
    if not candidates:
        return None
    return min(
        candidates,
        key=lambda event: abs(
            (_integer(_event_field(event, "gateway_entry_us", "gateway_entry")) or 0)
            - request.gateway_entry_us
        ),
    )


def _learned_point_us(event: Mapping[str, Any] | None) -> float | None:
    if event is None:
        return None
    return _number(
        _event_field(
            event,
            "learned_full_raw_us",
            "raw_learned_full_us",
            "learned_full_us",
            "learned_remaining_us",
            "estimate_us",
            "predicted_remaining_us",
        )
    )


def _learned_policy_us(event: Mapping[str, Any] | None) -> float | None:
    if event is None:
        return None
    effective = _number(
        _event_field(event, "effective_priority_us", "decayed_priority_us")
    )
    return effective if effective is not None else _learned_point_us(event)


def _reference_us(
    request: RequestResult, event: Mapping[str, Any] | None
) -> float | None:
    if event is not None:
        reference = _number(
            _event_field(
                event,
                "reference_remaining_us",
                "planned_remaining_after_us",
                "oracle_remaining_us",
            )
        )
        if reference is not None:
            return reference
    return request.plan_reference_us


def _effective_key(event: Mapping[str, Any] | None) -> str | None:
    if event is None:
        return None
    value = _event_field(
        event, "effective_priority_key", "effective_key", "lookup_key", "key"
    )
    return None if value is None else str(value)


def _priority_deadline(
    request: RequestResult, event: Mapping[str, Any] | None
) -> float:
    if event is None:
        return float(request.deadline_us)
    value = _number(
        _event_field(event, "parent_deadline_us", "e2e_deadline_us", "deadline_us")
    )
    return float(request.deadline_us) if value is None else value


def _distribution(values: Iterable[float]) -> dict[str, float | int | None]:
    materialized = [value for value in values if math.isfinite(value)]
    return {
        "count": len(materialized),
        "p50_us": percentile(materialized, 0.50),
        "p90_us": percentile(materialized, 0.90),
    }


def _request_metrics(
    requests: Sequence[RequestResult], duration_secs: float
) -> dict[str, Any]:
    on_time = [
        request
        for request in requests
        if not request.excluded_from_goodput and request.latency_us <= request.slo_us
    ]
    if requests:
        first_start_us = min(request.gateway_entry_us for request in requests)
        last_finish_us = max(
            request.gateway_entry_us + request.latency_us for request in requests
        )
        observed_duration_secs = (last_finish_us - first_start_us) / 1_000_000
        if observed_duration_secs > 0:
            duration_secs = observed_duration_secs
    classes: dict[str, Any] = {}
    for request_class in ("short", "long"):
        class_requests = [
            request for request in requests if request.request_class == request_class
        ]
        class_on_time = [
            request
            for request in class_requests
            if not request.excluded_from_goodput
            and request.latency_us <= request.slo_us
        ]
        classes[request_class] = {
            "count": len(class_requests),
            "slo_attainment": safe_fraction(len(class_on_time), len(class_requests)),
            "latency": _distribution(request.latency_us for request in class_requests),
            "shared_queue": _distribution(
                request.shared_queue_us
                for request in class_requests
                if request.shared_queue_us is not None
            ),
        }
    return {
        "requests": len(requests),
        "goodput_rps": safe_fraction(len(on_time), duration_secs),
        "slo_attainment": safe_fraction(len(on_time), len(requests)),
        "latency": _distribution(request.latency_us for request in requests),
        "shared_queue": _distribution(
            request.shared_queue_us
            for request in requests
            if request.shared_queue_us is not None
        ),
        "classes": classes,
    }


def _audit_accuracy(
    joined: Sequence[tuple[RequestResult, Mapping[str, Any] | None]],
) -> dict[str, Any]:
    errors: list[float] = []
    policy_errors: list[float] = []
    audit_count = 0
    keys: set[str] = set()
    for request, event in joined:
        learned = _learned_point_us(event)
        learned_policy = _learned_policy_us(event)
        reference = _reference_us(request, event)
        if event is not None:
            audit_count += 1
        key = _effective_key(event)
        if key is not None:
            keys.add(key)
        if learned is None or reference is None or request.slo_us <= 0:
            continue
        errors.append(abs(learned - reference) / request.slo_us)
        if learned_policy is not None:
            policy_errors.append(abs(learned_policy - reference) / request.slo_us)
    return {
        "available": audit_count > 0,
        "request_coverage": safe_fraction(audit_count, len(joined)),
        "effective_key_count": len(keys) if audit_count else None,
        "labeled_count": len(errors),
        "nae_p50": percentile(errors, 0.50),
        "nae_p90": percentile(errors, 0.90),
        "priority_nae_p50": percentile(policy_errors, 0.50),
        "priority_nae_p90": percentile(policy_errors, 0.90),
    }


def _ordering_metrics(
    joined: Sequence[tuple[RequestResult, Mapping[str, Any] | None]],
) -> dict[str, Any]:
    groups: dict[int, list[tuple[RequestResult, Mapping[str, Any] | None]]] = (
        defaultdict(list)
    )
    for item in joined:
        groups[item[0].deadline_us].append(item)

    learned_comparable = 0
    learned_agree = 0
    learned_ties = 0
    learned_pairs = 0
    cross_key_comparable = 0
    cross_key_flips = 0
    same_key_pairs = 0
    ambiguity_counts = {threshold: 0 for threshold in AMBIGUITY_THRESHOLDS}
    top_credits: list[float] = []
    top_ties = 0
    realized_start_comparable = 0
    realized_start_agree = 0
    realized_finish_comparable = 0
    realized_finish_agree = 0

    for group in groups.values():
        for (left, left_event), (right, right_event) in itertools.combinations(
            group, 2
        ):
            left_ref = _reference_us(left, left_event)
            right_ref = _reference_us(right, right_event)
            if left_ref is None or right_ref is None:
                continue
            left_key = _effective_key(left_event)
            right_key = _effective_key(right_event)
            if left_key is not None and left_key == right_key:
                same_key_pairs += 1
                difference = abs(left_ref - right_ref)
                pair_slo = min(left.slo_us, right.slo_us)
                for threshold in AMBIGUITY_THRESHOLDS:
                    ambiguity_counts[threshold] += difference > threshold * pair_slo

            left_ref_score = _priority_deadline(left, left_event) - left_ref
            right_ref_score = _priority_deadline(right, right_event) - right_ref
            ref_order = _cmp(left_ref_score, right_ref_score)
            if ref_order == 0:
                continue

            if (
                left.shared_started_at_us is not None
                and right.shared_started_at_us is not None
            ):
                realized_start_comparable += 1
                realized_start_agree += (
                    _cmp(left.shared_started_at_us, right.shared_started_at_us)
                    == ref_order
                )
            if (
                left.shared_finished_at_us is not None
                and right.shared_finished_at_us is not None
            ):
                realized_finish_comparable += 1
                realized_finish_agree += (
                    _cmp(left.shared_finished_at_us, right.shared_finished_at_us)
                    == ref_order
                )

            left_learned = _learned_policy_us(left_event)
            right_learned = _learned_policy_us(right_event)
            if left_learned is None or right_learned is None:
                continue
            learned_pairs += 1
            left_score = _priority_deadline(left, left_event) - left_learned
            right_score = _priority_deadline(right, right_event) - right_learned
            learned_order = _cmp(left_score, right_score)
            if learned_order == 0:
                learned_ties += 1
            else:
                learned_comparable += 1
                learned_agree += learned_order == ref_order
                if (
                    left_key is not None
                    and right_key is not None
                    and left_key != right_key
                ):
                    cross_key_comparable += 1
                    cross_key_flips += learned_order != ref_order

        eligible = [
            (request, event)
            for request, event in group
            if _reference_us(request, event) is not None
            and _learned_policy_us(event) is not None
        ]
        if len(eligible) < 2:
            continue
        ref_scores = [
            _priority_deadline(request, event) - float(_reference_us(request, event))
            for request, event in eligible
        ]
        learned_scores = [
            _priority_deadline(request, event) - float(_learned_policy_us(event))
            for request, event in eligible
        ]
        reference_top = {
            index for index, score in enumerate(ref_scores) if score == min(ref_scores)
        }
        learned_top = {
            index
            for index, score in enumerate(learned_scores)
            if score == min(learned_scores)
        }
        # Fractional credit is the probability that an arbitrary scheduler
        # tie-break among learned top candidates selects a reference-top item.
        top_credits.append(len(reference_top & learned_top) / len(learned_top))
        top_ties += len(learned_top) > 1

    return {
        "matched_deadline_groups": len(groups),
        "learned_pair_count": learned_pairs,
        "learned_pair_tie_fraction": safe_fraction(learned_ties, learned_pairs),
        "pairwise_reference_order_agreement": safe_fraction(
            learned_agree, learned_comparable
        ),
        "pairwise_agreement_comparable_pairs": learned_comparable,
        "top_choice_reference_agreement": (
            statistics.fmean(top_credits) if top_credits else None
        ),
        "learned_top_tie_fraction": safe_fraction(top_ties, len(top_credits)),
        "cross_key_flip_fraction": safe_fraction(cross_key_flips, cross_key_comparable),
        "cross_key_comparable_pairs": cross_key_comparable,
        "same_key_pairs": same_key_pairs,
        "same_key_ambiguity": {
            f"gt_{int(threshold * 100)}pct_slo": safe_fraction(
                ambiguity_counts[threshold], same_key_pairs
            )
            for threshold in AMBIGUITY_THRESHOLDS
        },
        "realized_shared_start_reference_agreement": safe_fraction(
            realized_start_agree, realized_start_comparable
        ),
        "realized_shared_finish_reference_agreement": safe_fraction(
            realized_finish_agree, realized_finish_comparable
        ),
    }


def _confusion(predictions: Sequence[tuple[bool, bool]]) -> dict[str, Any]:
    tp = sum(predicted and actual for predicted, actual in predictions)
    fp = sum(predicted and not actual for predicted, actual in predictions)
    tn = sum(not predicted and not actual for predicted, actual in predictions)
    fn = sum(not predicted and actual for predicted, actual in predictions)
    return {
        "count": len(predictions),
        "tp": tp,
        "fp": fp,
        "tn": tn,
        "fn": fn,
        "precision": safe_fraction(tp, tp + fp),
        "recall": safe_fraction(tp, tp + fn),
        "false_positive_rate": safe_fraction(fp, fp + tn),
    }


def _shadow_feasibility(
    joined: Sequence[tuple[RequestResult, Mapping[str, Any] | None]],
) -> dict[str, Any]:
    learned_predictions: list[tuple[bool, bool]] = []
    reference_predictions: list[tuple[bool, bool]] = []
    for request, event in joined:
        if event is None:
            continue
        time_left = _number(_event_field(event, "time_left_us"))
        learned = _learned_point_us(event)
        reference = _reference_us(request, event)
        # This experiment intentionally labels only final latency>SLO. Errors
        # are retained in request-level goodput but do not redefine this label.
        missed_slo = request.latency_us > request.slo_us
        if learned is not None:
            learned_deadline = _number(_event_field(event, "child_deadline_us"))
            if (
                learned_deadline is not None
                and request.shared_finished_at_us is not None
            ):
                learned_late = request.shared_finished_at_us > learned_deadline
                learned_predictions.append((learned_late, missed_slo))
            elif time_left is not None:
                # Backward-compatible fallback for early audit records that
                # did not log the tightened child deadline.
                learned_predictions.append((time_left < learned, missed_slo))
        if reference is not None:
            parent_deadline = _number(
                _event_field(event, "parent_deadline_us", "e2e_deadline_us")
            )
            if (
                parent_deadline is not None
                and request.shared_finished_at_us is not None
            ):
                reference_deadline = max(0.0, parent_deadline - reference)
                reference_late = request.shared_finished_at_us > reference_deadline
                reference_predictions.append((reference_late, missed_slo))
            elif time_left is not None:
                reference_predictions.append((time_left < reference, missed_slo))
    return {
        "learned": _confusion(learned_predictions),
        "reference": _confusion(reference_predictions),
    }


def _duration(policy_dir: Path, fallback: Path) -> float:
    for path in (policy_dir / "gen_config.json", fallback / "gen_config.json"):
        if not path.exists():
            continue
        with path.open(encoding="utf-8") as config_file:
            value = _number(json.load(config_file).get("DurationSecs"))
        if value is not None and value > 0:
            return value
    return 1.0


def _experiment_roots(paths: Sequence[Path]) -> list[Path]:
    roots: set[Path] = set()
    for path in paths:
        if not path.exists() or not path.is_dir():
            continue
        if any(child.is_dir() and child.name.isdigit() for child in path.iterdir()):
            roots.add(path)
            continue
        roots.update(
            child
            for child in path.glob("estimator_ordering*")
            if child.is_dir()
            and any(
                grandchild.is_dir() and grandchild.name.isdigit()
                for grandchild in child.iterdir()
            )
        )
    return sorted(roots)


def analyze(paths: Sequence[Path]) -> dict[str, Any]:
    runs: list[dict[str, Any]] = []
    for root in _experiment_roots(paths):
        for iteration_dir in sorted(
            (
                child
                for child in root.iterdir()
                if child.is_dir() and child.name.isdigit()
            ),
            key=lambda child: int(child.name),
        ):
            for policy_dir in sorted(
                child for child in iteration_dir.iterdir() if child.is_dir()
            ):
                rps_values = sorted(
                    {
                        int(match.group("rps"))
                        for file_path in policy_dir.glob("r*_*.csv")
                        if (match := RESULT_FILE.fullmatch(file_path.name))
                    }
                )
                if not rps_values:
                    continue
                audit_events = parse_audit_logs(policy_dir)
                audit_index = _audit_index(audit_events)
                for rps in rps_values:
                    requests = _load_request_results(policy_dir, rps)
                    joined = [
                        (request, _choose_audit_event(audit_index, request))
                        for request in requests
                    ]
                    runs.append(
                        {
                            "experiment": root.name,
                            "iteration": int(iteration_dir.name),
                            "policy": policy_dir.name,
                            "rps": rps,
                            **_request_metrics(requests, _duration(policy_dir, root)),
                            "audit": _audit_accuracy(joined),
                            "ordering": _ordering_metrics(joined),
                            "shadow_feasibility": _shadow_feasibility(joined),
                        }
                    )
    return {"runs": runs, "summary": aggregate_runs(runs)}


SUMMARY_FIELDS: dict[str, tuple[str, ...]] = {
    "goodput_rps": ("goodput_rps",),
    "latency_p50_us": ("latency", "p50_us"),
    "latency_p90_us": ("latency", "p90_us"),
    "shared_queue_p50_us": ("shared_queue", "p50_us"),
    "shared_queue_p90_us": ("shared_queue", "p90_us"),
    "short_slo_attainment": ("classes", "short", "slo_attainment"),
    "long_slo_attainment": ("classes", "long", "slo_attainment"),
    "audit_coverage": ("audit", "request_coverage"),
    "nae_p50": ("audit", "nae_p50"),
    "nae_p90": ("audit", "nae_p90"),
    "priority_nae_p50": ("audit", "priority_nae_p50"),
    "priority_nae_p90": ("audit", "priority_nae_p90"),
    "learned_tie_fraction": ("ordering", "learned_pair_tie_fraction"),
    "pairwise_agreement": ("ordering", "pairwise_reference_order_agreement"),
    "top_choice_agreement": ("ordering", "top_choice_reference_agreement"),
    "cross_key_flip_fraction": ("ordering", "cross_key_flip_fraction"),
    "same_key_ambiguity_1pct": (
        "ordering",
        "same_key_ambiguity",
        "gt_1pct_slo",
    ),
    "same_key_ambiguity_5pct": (
        "ordering",
        "same_key_ambiguity",
        "gt_5pct_slo",
    ),
    "same_key_ambiguity_10pct": (
        "ordering",
        "same_key_ambiguity",
        "gt_10pct_slo",
    ),
    "same_key_ambiguity_25pct": (
        "ordering",
        "same_key_ambiguity",
        "gt_25pct_slo",
    ),
    "realized_start_agreement": (
        "ordering",
        "realized_shared_start_reference_agreement",
    ),
    "realized_finish_agreement": (
        "ordering",
        "realized_shared_finish_reference_agreement",
    ),
    "shadow_learned_precision": ("shadow_feasibility", "learned", "precision"),
    "shadow_learned_recall": ("shadow_feasibility", "learned", "recall"),
    "shadow_learned_fpr": (
        "shadow_feasibility",
        "learned",
        "false_positive_rate",
    ),
    "shadow_reference_precision": (
        "shadow_feasibility",
        "reference",
        "precision",
    ),
    "shadow_reference_recall": ("shadow_feasibility", "reference", "recall"),
    "shadow_reference_fpr": (
        "shadow_feasibility",
        "reference",
        "false_positive_rate",
    ),
}


def _nested(record: Mapping[str, Any], path: Sequence[str]) -> float | None:
    value: Any = record
    for component in path:
        if not isinstance(value, Mapping) or component not in value:
            return None
        value = value[component]
    return _number(value)


T_CRITICAL_975 = {
    1: 12.706,
    2: 4.303,
    3: 3.182,
    4: 2.776,
    5: 2.571,
    6: 2.447,
    7: 2.365,
    8: 2.306,
    9: 2.262,
    10: 2.228,
}


def _mean_ci(values: Sequence[float]) -> dict[str, float | int | None]:
    if not values:
        return {"mean": None, "ci95": None, "n": 0}
    mean = statistics.fmean(values)
    if len(values) < 2:
        return {"mean": mean, "ci95": None, "n": len(values)}
    standard_error = statistics.stdev(values) / math.sqrt(len(values))
    critical = T_CRITICAL_975.get(len(values) - 1, 1.96)
    return {"mean": mean, "ci95": critical * standard_error, "n": len(values)}


def aggregate_runs(runs: Sequence[Mapping[str, Any]]) -> list[dict[str, Any]]:
    grouped: dict[tuple[str, str, int], list[Mapping[str, Any]]] = defaultdict(list)
    for run in runs:
        grouped[(str(run["experiment"]), str(run["policy"]), int(run["rps"]))].append(
            run
        )
    summary: list[dict[str, Any]] = []
    for (experiment, policy, rps), group in sorted(grouped.items()):
        metrics = {
            label: _mean_ci(
                [value for run in group if (value := _nested(run, path)) is not None]
            )
            for label, path in SUMMARY_FIELDS.items()
        }
        summary.append(
            {
                "experiment": experiment,
                "policy": policy,
                "rps": rps,
                "iterations": len(group),
                "metrics": metrics,
            }
        )
    return summary


def _fmt(metric: Mapping[str, Any], percent: bool = False) -> str:
    mean = _number(metric.get("mean"))
    ci = _number(metric.get("ci95"))
    if mean is None:
        return "--"
    scale = 100.0 if percent else 1.0
    if ci is None:
        return f"{mean * scale:.3f}"
    return f"{mean * scale:.3f} ± {ci * scale:.3f}"


def _fmt_ms(metric: Mapping[str, Any]) -> str:
    mean = _number(metric.get("mean"))
    ci = _number(metric.get("ci95"))
    if mean is None:
        return "--"
    if ci is None:
        return f"{mean / 1_000:.3f}"
    return f"{mean / 1_000:.3f} ± {ci / 1_000:.3f}"


def markdown_summary(summary: Sequence[Mapping[str, Any]]) -> str:
    outcome_columns = (
        "Experiment",
        "Policy",
        "RPS",
        "Goodput",
        "Latency p50/p90 ms",
        "Queue p50/p90 ms",
        "Short SLO",
        "Long SLO",
    )
    lines = [
        "### Outcomes",
        "",
        "| " + " | ".join(outcome_columns) + " |",
        "| "
        + " | ".join(["---", "---", "---:"] + ["---:"] * (len(outcome_columns) - 3))
        + " |",
    ]
    for row in summary:
        metrics = row["metrics"]
        values = (
            str(row["experiment"]),
            str(row["policy"]),
            str(row["rps"]),
            _fmt(metrics["goodput_rps"]),
            f"{_fmt_ms(metrics['latency_p50_us'])}/{_fmt_ms(metrics['latency_p90_us'])}",
            f"{_fmt_ms(metrics['shared_queue_p50_us'])}/{_fmt_ms(metrics['shared_queue_p90_us'])}",
            _fmt(metrics["short_slo_attainment"], percent=True),
            _fmt(metrics["long_slo_attainment"], percent=True),
        )
        lines.append("| " + " | ".join(values) + " |")

    audit_columns = (
        "Experiment",
        "Policy",
        "RPS",
        "NAE p50/p90",
        "Policy NAE p50/p90",
        "Pair agree",
        "Top agree",
        "Ties",
        "Cross flips",
        "Same-key >5%",
    )
    lines.extend(
        [
            "",
            "### Estimator ordering",
            "",
            "| " + " | ".join(audit_columns) + " |",
            "| "
            + " | ".join(["---", "---", "---:"] + ["---:"] * (len(audit_columns) - 3))
            + " |",
        ]
    )
    for row in summary:
        metrics = row["metrics"]
        nae = f"{_fmt(metrics['nae_p50'])}/{_fmt(metrics['nae_p90'])}"
        policy_nae = (
            f"{_fmt(metrics['priority_nae_p50'])}/"
            f"{_fmt(metrics['priority_nae_p90'])}"
        )
        values = (
            str(row["experiment"]),
            str(row["policy"]),
            str(row["rps"]),
            nae,
            policy_nae,
            _fmt(metrics["pairwise_agreement"], percent=True),
            _fmt(metrics["top_choice_agreement"], percent=True),
            _fmt(metrics["learned_tie_fraction"], percent=True),
            _fmt(metrics["cross_key_flip_fraction"], percent=True),
            _fmt(metrics["same_key_ambiguity_5pct"], percent=True),
        )
        lines.append("| " + " | ".join(values) + " |")

    shadow_columns = (
        "Experiment",
        "Policy",
        "RPS",
        "Learned P/R/FPR",
        "Reference P/R/FPR",
    )
    lines.extend(
        [
            "",
            "### Shadow feasibility",
            "",
            "| " + " | ".join(shadow_columns) + " |",
            "| "
            + " | ".join(["---", "---", "---:"] + ["---:"] * (len(shadow_columns) - 3))
            + " |",
        ]
    )
    for row in summary:
        metrics = row["metrics"]
        values = (
            str(row["experiment"]),
            str(row["policy"]),
            str(row["rps"]),
            "/".join(
                _fmt(metrics[name], percent=True)
                for name in (
                    "shadow_learned_precision",
                    "shadow_learned_recall",
                    "shadow_learned_fpr",
                )
            ),
            "/".join(
                _fmt(metrics[name], percent=True)
                for name in (
                    "shadow_reference_precision",
                    "shadow_reference_recall",
                    "shadow_reference_fpr",
                )
            ),
        )
        lines.append("| " + " | ".join(values) + " |")
    return "\n".join(lines) + "\n"


def write_outputs(result: Mapping[str, Any], output_dir: Path) -> None:
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "summary.json").write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    summary = result["summary"]
    fieldnames = ["experiment", "policy", "rps", "iterations"] + [
        suffix
        for metric in SUMMARY_FIELDS
        for suffix in (f"{metric}_mean", f"{metric}_ci95", f"{metric}_n")
    ]
    with (output_dir / "summary.csv").open(
        "w", newline="", encoding="utf-8"
    ) as csv_file:
        writer = csv.DictWriter(csv_file, fieldnames=fieldnames, lineterminator="\n")
        writer.writeheader()
        for item in summary:
            row: dict[str, Any] = {
                "experiment": item["experiment"],
                "policy": item["policy"],
                "rps": item["rps"],
                "iterations": item["iterations"],
            }
            for metric, values in item["metrics"].items():
                row[f"{metric}_mean"] = values["mean"]
                row[f"{metric}_ci95"] = values["ci95"]
                row[f"{metric}_n"] = values["n"]
            writer.writerow(row)
    markdown = markdown_summary(summary)
    (output_dir / "summary.md").write_text(markdown, encoding="utf-8")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "paths",
        nargs="*",
        type=Path,
        default=[Path("exp/synthbench/out")],
        help="Experiment output roots or their common parent",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        default=Path("eval/estimator_ordering/results"),
    )
    args = parser.parse_args()
    result = analyze(args.paths)
    write_outputs(result, args.output_dir)
    print(markdown_summary(result["summary"]), end="")


if __name__ == "__main__":
    main()
