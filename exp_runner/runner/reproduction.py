"""Semantic declarations for reproducible evaluation studies.

The existing experiment runner organizes inputs by application and directory
name.  Reproduction manifests add a stable, scientific identity above that
layout and deliberately keep manuscript figure numbers out of the model.
"""

from __future__ import annotations

import re
from dataclasses import dataclass, replace
from pathlib import Path
from typing import Any

import yaml

from .apps import get_app_plugin
from .config import ExperimentConfig


_SEMANTIC_ID = re.compile(r"^[a-z0-9][a-z0-9_/-]*$")


class ReproductionManifestError(ValueError):
    """Raised when a reproduction declaration is invalid or has drifted."""


def _load_yaml(path: Path) -> dict[str, Any]:
    if not path.is_file():
        raise ReproductionManifestError(f"Manifest not found: {path}")
    with path.open(encoding="utf-8") as handle:
        value = yaml.safe_load(handle)
    if not isinstance(value, dict):
        raise ReproductionManifestError(f"Expected a YAML mapping in {path}")
    return value


def _mapping(value: Any, field: str, path: Path) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ReproductionManifestError(f"{path}: '{field}' must be a mapping")
    return value


def _string(value: Any, field: str, path: Path) -> str:
    if not isinstance(value, str) or not value.strip():
        raise ReproductionManifestError(f"{path}: '{field}' must be a non-empty string")
    return value.strip()


def _string_list(value: Any, field: str, path: Path) -> list[str]:
    if not isinstance(value, list) or not value:
        raise ReproductionManifestError(f"{path}: '{field}' must be a non-empty list")
    result = []
    for item in value:
        result.append(_string(item, field, path))
    return result


def _int_list(value: Any, field: str, path: Path) -> list[int]:
    if not isinstance(value, list) or not value:
        raise ReproductionManifestError(f"{path}: '{field}' must be a non-empty list")
    if any(not isinstance(item, int) or isinstance(item, bool) for item in value):
        raise ReproductionManifestError(f"{path}: '{field}' must contain integers")
    return list(value)


def _positive_int(
    value: Any, field: str, path: Path, *, allow_zero: bool = False
) -> int:
    minimum = 0 if allow_zero else 1
    if not isinstance(value, int) or isinstance(value, bool) or value < minimum:
        qualifier = "non-negative" if allow_zero else "positive"
        raise ReproductionManifestError(
            f"{path}: '{field}' must be a {qualifier} integer"
        )
    return value


def _semantic_id(value: Any, field: str, path: Path) -> str:
    result = _string(value, field, path)
    if not _SEMANTIC_ID.fullmatch(result) or "//" in result:
        raise ReproductionManifestError(
            f"{path}: '{field}' must be a lowercase semantic ID"
        )
    return result


def _canonical_policy(features: list[str]) -> frozenset[str]:
    return frozenset(feature.strip() for feature in features if feature.strip())


@dataclass(frozen=True)
class PolicyProfile:
    """A reusable semantic name for one compile-time policy configuration."""

    id: str
    description: str
    features: tuple[str, ...]
    source_path: Path

    @classmethod
    def load(cls, path: Path) -> "PolicyProfile":
        data = _load_yaml(path)
        if data.get("version") != 1:
            raise ReproductionManifestError(f"{path}: unsupported policy version")
        return cls(
            id=_semantic_id(data.get("id"), "id", path),
            description=_string(data.get("description"), "description", path),
            features=tuple(_string_list(data.get("features"), "features", path)),
            source_path=path,
        )

    @property
    def runner_policy(self) -> str:
        return ",".join(self.features)


@dataclass(frozen=True)
class RunnerBinding:
    """Adapter from a semantic declaration to the current experiment runner."""

    app: str
    experiment: str


@dataclass(frozen=True)
class ReproductionExperiment:
    """One semantically named experiment and its executable runner binding."""

    id: str
    description: str
    hypothesis: str
    workload_callgraphs: tuple[str, ...]
    workload_slo_ms: int
    arrival_distribution: str
    offered_rps: tuple[int, ...]
    warmup_secs: int
    measurement_secs: int
    repetitions: int
    max_in_flight: int
    policies: tuple[PolicyProfile, ...]
    policy_parameters: dict[str, Any]
    primary_metric: str
    comparisons: tuple[str, ...]
    runner: RunnerBinding
    source_path: Path
    study_id: str = ""

    @classmethod
    def load(cls, path: Path, suite_root: Path) -> "ReproductionExperiment":
        data = _load_yaml(path)
        if data.get("version") != 1:
            raise ReproductionManifestError(f"{path}: unsupported experiment version")

        workload = _mapping(data.get("workload"), "workload", path)
        arrival = _mapping(data.get("load"), "load", path)
        execution = _mapping(data.get("execution"), "execution", path)
        analysis = _mapping(data.get("analysis"), "analysis", path)
        runner_data = _mapping(data.get("runner"), "runner", path)

        policy_refs = _string_list(data.get("policies"), "policies", path)
        policies = tuple(
            PolicyProfile.load((suite_root / policy_ref).resolve())
            for policy_ref in policy_refs
        )
        policy_ids = [policy.id for policy in policies]
        if len(policy_ids) != len(set(policy_ids)):
            raise ReproductionManifestError(f"{path}: duplicate policy IDs")

        comparisons = analysis.get("comparisons", [])
        if not isinstance(comparisons, list) or any(
            not isinstance(value, str) or not value.strip() for value in comparisons
        ):
            raise ReproductionManifestError(
                f"{path}: 'analysis.comparisons' must be a list of strings"
            )

        return cls(
            id=_semantic_id(data.get("id"), "id", path),
            description=_string(data.get("description"), "description", path),
            hypothesis=_string(data.get("hypothesis"), "hypothesis", path),
            workload_callgraphs=tuple(
                _string_list(workload.get("callgraphs"), "workload.callgraphs", path)
            ),
            workload_slo_ms=_positive_int(
                workload.get("slo_ms"), "workload.slo_ms", path
            ),
            arrival_distribution=_string(arrival.get("arrival"), "load.arrival", path),
            offered_rps=tuple(
                _int_list(arrival.get("offered_rps"), "load.offered_rps", path)
            ),
            warmup_secs=_positive_int(
                execution.get("warmup_secs"),
                "execution.warmup_secs",
                path,
                allow_zero=True,
            ),
            measurement_secs=_positive_int(
                execution.get("measurement_secs"),
                "execution.measurement_secs",
                path,
            ),
            repetitions=_positive_int(
                execution.get("repetitions"), "execution.repetitions", path
            ),
            max_in_flight=_positive_int(
                execution.get("max_in_flight", 0),
                "execution.max_in_flight",
                path,
                allow_zero=True,
            ),
            policies=policies,
            policy_parameters=_mapping(data.get("parameters"), "parameters", path),
            primary_metric=_string(
                analysis.get("primary_metric"), "analysis.primary_metric", path
            ),
            comparisons=tuple(value.strip() for value in comparisons),
            runner=RunnerBinding(
                app=_string(runner_data.get("app"), "runner.app", path),
                experiment=_string(
                    runner_data.get("experiment"), "runner.experiment", path
                ),
            ),
            source_path=path,
        )

    def with_study(self, study_id: str) -> "ReproductionExperiment":
        return replace(self, study_id=study_id)

    def validate_runner_binding(self, repo_root: Path) -> ExperimentConfig:
        """Ensure the executable legacy config has not drifted from this declaration."""

        try:
            app_plugin = get_app_plugin(self.runner.app)
            config = ExperimentConfig.load(
                experiment_name=self.runner.experiment,
                app_name=self.runner.app,
                repo_root=repo_root,
                app_plugin=app_plugin,
            )
        except (FileNotFoundError, ValueError) as error:
            raise ReproductionManifestError(
                f"{self.id}: invalid runner binding: {error}"
            ) from error

        expected_gen = {
            "Rps": list(self.offered_rps),
            "WarmupSecs": self.warmup_secs,
            "DurationSecs": self.measurement_secs,
            "Repeats": self.repetitions,
            "MaxInFlight": self.max_in_flight,
        }
        for field, expected in expected_gen.items():
            actual = config.gen_config.get(field, 0 if field == "MaxInFlight" else None)
            if actual != expected:
                raise ReproductionManifestError(
                    f"{self.id}: runner {field} is {actual!r}, expected {expected!r}"
                )

        app_config = config.app_config or {}
        actual_graphs = app_config.get("callgraph_dirs")
        if actual_graphs != list(self.workload_callgraphs):
            raise ReproductionManifestError(
                f"{self.id}: runner callgraphs are {actual_graphs!r}, "
                f"expected {list(self.workload_callgraphs)!r}"
            )
        if app_config.get("slo_ms") != self.workload_slo_ms:
            raise ReproductionManifestError(
                f"{self.id}: runner SLO is {app_config.get('slo_ms')!r} ms, "
                f"expected {self.workload_slo_ms} ms"
            )

        expected_policies = {
            _canonical_policy(list(policy.features)) for policy in self.policies
        }
        actual_policies = {
            _canonical_policy(policy.split(",")) for policy in config.policies
        }
        if actual_policies != expected_policies:
            raise ReproductionManifestError(
                f"{self.id}: runner policies do not match semantic policy profiles"
            )

        if config.policy_params != self.policy_parameters:
            raise ReproductionManifestError(
                f"{self.id}: runner policy parameters do not match the semantic "
                "declaration"
            )
        return config


@dataclass(frozen=True)
class ReproductionStudy:
    id: str
    question: str
    experiments: tuple[ReproductionExperiment, ...]


@dataclass(frozen=True)
class ReproductionSuite:
    """A collection of studies organized by scientific question."""

    name: str
    studies: tuple[ReproductionStudy, ...]
    source_path: Path

    @classmethod
    def load(cls, path: Path, repo_root: Path) -> "ReproductionSuite":
        path = path.resolve()
        data = _load_yaml(path)
        if data.get("version") != 1:
            raise ReproductionManifestError(f"{path}: unsupported suite version")
        suite_root = path.parent
        studies_data = _mapping(data.get("studies"), "studies", path)
        if not studies_data:
            raise ReproductionManifestError(f"{path}: suite contains no studies")

        studies = []
        experiment_ids: set[str] = set()
        for raw_study_id, raw_study in studies_data.items():
            study_id = _semantic_id(raw_study_id, "study ID", path)
            study_data = _mapping(raw_study, f"studies.{study_id}", path)
            refs = _string_list(
                study_data.get("experiments"),
                f"studies.{study_id}.experiments",
                path,
            )
            experiments = []
            for experiment_ref in refs:
                experiment = ReproductionExperiment.load(
                    (suite_root / experiment_ref).resolve(), suite_root
                ).with_study(study_id)
                if experiment.id in experiment_ids:
                    raise ReproductionManifestError(
                        f"{path}: duplicate experiment ID '{experiment.id}'"
                    )
                experiment.validate_runner_binding(repo_root)
                experiment_ids.add(experiment.id)
                experiments.append(experiment)
            studies.append(
                ReproductionStudy(
                    id=study_id,
                    question=_string(
                        study_data.get("question"),
                        f"studies.{study_id}.question",
                        path,
                    ),
                    experiments=tuple(experiments),
                )
            )
        return cls(
            name=_string(data.get("name"), "name", path),
            studies=tuple(studies),
            source_path=path,
        )

    @property
    def experiments(self) -> tuple[ReproductionExperiment, ...]:
        return tuple(
            experiment for study in self.studies for experiment in study.experiments
        )

    def select(self, semantic_id: str | None) -> tuple[ReproductionExperiment, ...]:
        if semantic_id is None:
            return self.experiments
        selected = tuple(
            experiment
            for experiment in self.experiments
            if experiment.id == semantic_id
        )
        if not selected:
            known = ", ".join(experiment.id for experiment in self.experiments)
            raise ReproductionManifestError(
                f"Unknown experiment '{semantic_id}'. Available: {known}"
            )
        return selected
