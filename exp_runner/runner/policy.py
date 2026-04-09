"""Parsed representation of a Masa scheduling policy.

A policy is specified as a comma-separated string of Cargo feature flags
(e.g. "sched_pred,abort_slo,est_mean_var"). This module parses that string
into a structured ``Policy`` object with typed fields for each category.
"""

from __future__ import annotations

from dataclasses import dataclass

# Default estimator type when a slack-related flag is present but no explicit
# est_* flag is given.  Must stay in sync with the Rust compile-time default
# in libs/masa-policy/src/layer/est/estimator.rs (LatencyMeanVar fallback).
DEFAULT_EST = "mean_var"

# ── flag → (field, value) mapping ──────────────────────────────────────────

_PRIO_MAP: dict[str, str] = {
    "sched_fifo": "fifo",
    "sched_slo": "e2e_slo",
    "sched_tailclipper": "oldest",
    "sched_pred": "slack",
}

# sched_pred and sched_tailclipper imply sched_slo at the Cargo level.
# When both are present, sched_slo is redundant.
_PRIO_PRIORITY = ["sched_pred", "sched_tailclipper", "sched_slo", "sched_fifo"]

_EST_MAP: dict[str, str] = {
    "est_mean_var": "mean_var",
    "est_hist": "hist",
    "est_rms": "rms",
}

_DROP_MAP: dict[str, str] = {
    "abort_slack": "slack",
    "abort_slo": "e2e_slo",
}

_AC_MAP: dict[str, str] = {
    "ac_pred": "slack",
    "ac_rajomon": "rajomon",
}

# Flags that are always ignored (implied by other flags).
_IGNORED_FLAGS = {"sched_slo", "estimator"}


def _parse_flags(policy: str) -> frozenset[str]:
    """Split a comma-separated policy string into a set of flags."""
    return frozenset(tok for raw in policy.split(",") if (tok := raw.strip().lower()))


@dataclass(frozen=True)
class Policy:
    """Parsed representation of a comma-separated policy feature flag string.

    Fields:
        raw:  Original string (used as folder name / dict key).
        prio: Scheduling priority — "fifo", "e2e_slo", "oldest", or "slack".
        est:  Estimator type — "mean_var", "hist", or "rms".  Only set when a
              slack-related flag is present (prio=slack or ac=slack).
        drop: Drop mechanism — "e2e_slo" (abort past-SLO requests).
        ac:   Admission control — "slack" or "rajomon".
    """

    raw: str
    prio: str | None
    est: str | None
    drop: str | None
    ac: str | None

    # ── construction ───────────────────────────────────────────────────

    @staticmethod
    def parse(policy: str) -> Policy:
        """Parse a comma-separated feature-flag string into a Policy."""
        flags = _parse_flags(policy)

        # Scheduling priority: pick the highest-priority flag present.
        prio: str | None = None
        for flag in _PRIO_PRIORITY:
            if flag in flags:
                prio = _PRIO_MAP[flag]
                break

        # Drop mechanism.
        drop: str | None = None
        for flag, val in _DROP_MAP.items():
            if flag in flags:
                drop = val
                break

        # Admission control.
        ac: str | None = None
        for flag, val in _AC_MAP.items():
            if flag in flags:
                ac = val
                break

        # Estimator — only relevant when any slack-related flag is present.
        uses_slack = prio == "slack" or ac == "slack"
        est: str | None = None
        if uses_slack:
            for flag, val in _EST_MAP.items():
                if flag in flags:
                    est = val
                    break
            if est is None:
                est = DEFAULT_EST

        # If we didn't recognise any scheduling flag, return an unparsed Policy.
        if prio is None:
            return Policy(raw=policy, prio=None, est=None, drop=None, ac=None)

        return Policy(raw=policy, prio=prio, est=est, drop=drop, ac=ac)

    # ── display ────────────────────────────────────────────────────────

    @property
    def display_name(self) -> str:
        """Human-readable key=value display name.

        Token order: prio → drop → ac [→ est (only for slack policies)].
        Unrecognised policies fall back to the raw string.
        """
        if self.prio is None:
            return self.raw

        parts = [
            f"prio={self.prio}",
            f"drop={self.drop or 'none'}",
            f"ac={self.ac or 'none'}",
        ]
        if self.est is not None:
            parts.append(f"est={self.est}")
        return ", ".join(parts)

    @property
    def color(self) -> str | None:
        """Matplotlib color for this policy, or None for the default cycle.

        Colour-blind-safe scheme (Okabe-Ito palette). Encodes prio + ac family;
        the `drop` dimension is encoded by `linestyle` instead of a lighter shade.

        - fifo:    grey         (#999999)
        - e2e_slo: blue         (#0072B2)
        - oldest:  reddish purple (#CC79A7)
        - slack:   vermillion   (#D55E00)  — no AC
                   bluish green (#009E73)  — ac=slack (cost-aware AC)
                   orange       (#E69F00)  — ac=rajomon
        """
        if self.prio == "fifo":
            return "#999999"
        if self.prio == "e2e_slo":
            if self.ac == "slack":
                return "#009E73"
            if self.ac == "rajomon":
                return "#E69F00"
            return "#0072B2"
        if self.prio == "oldest":
            return "#CC79A7"
        if self.prio == "slack":
            if self.ac == "slack":
                return "#009E73"
            if self.ac == "rajomon":
                return "#E69F00"
            return "#D55E00"
        return None

    @property
    def marker(self) -> str:
        """Matplotlib marker shape for this policy. Redundant encoding of `prio`
        so categories survive greyscale printing and CVD viewers."""
        if self.prio == "fifo":
            return "o"
        if self.prio == "e2e_slo":
            return "s"
        if self.prio == "oldest":
            return "^"
        if self.prio == "slack":
            return "D"
        return "o"

    @property
    def linestyle(self) -> str:
        """Matplotlib linestyle. Encodes the `drop` dimension orthogonally:
        solid = no drop, dashed = any drop variant."""
        return "--" if self.drop is not None else "-"

    @property
    def hatch(self) -> str:
        """Matplotlib hatch pattern for bar plots. Redundant encoding of `prio`."""
        if self.prio == "fifo":
            return ""
        if self.prio == "e2e_slo":
            return "//"
        if self.prio == "oldest":
            return "xx"
        if self.prio == "slack":
            return ".."
        return ""
