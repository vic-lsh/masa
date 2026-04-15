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

        Encodes (prio, drop) so that any two policies differing in scheduling
        priority or abort mechanism get distinct colors.  The AC dimension is
        encoded by `marker` instead.

        Palette (Okabe-Ito color-blind-safe + grey):

          prio=fifo:
            no drop     → grey          #999999
            abort_slo   → sky blue      #56B4E9
            abort_slack → blue          #0072B2

          prio=e2e_slo:
            no drop     → yellow        #F0E442
            abort_slo   → black         #000000
            abort_slack → light blue    #88CCEE  (rare)

          prio=oldest (tailclipper):
            no drop     → dark pink     #AA3377  (rare)
            abort_slo   → reddish purple #CC79A7
            abort_slack → dark wine     #882255  (rare)

          prio=slack (pred):
            no drop     → vermillion    #D55E00
            abort_slo   → orange        #E69F00
            abort_slack → bluish green  #009E73
        """
        _color_map: dict[tuple[str | None, str | None], str] = {
            ("fifo", None): "#999999",
            ("fifo", "e2e_slo"): "#56B4E9",
            ("fifo", "slack"): "#0072B2",
            ("e2e_slo", None): "#F0E442",
            ("e2e_slo", "e2e_slo"): "#000000",
            ("e2e_slo", "slack"): "#88CCEE",
            ("oldest", None): "#AA3377",
            ("oldest", "e2e_slo"): "#CC79A7",
            ("oldest", "slack"): "#882255",
            ("slack", None): "#D55E00",
            ("slack", "e2e_slo"): "#E69F00",
            ("slack", "slack"): "#009E73",
        }
        return _color_map.get((self.prio, self.drop))

    @property
    def marker(self) -> str:
        """Matplotlib marker shape for this policy.

        Encodes the admission control mechanism so that policies with the same
        (prio, drop) but different AC are visually distinct:
          no AC       → circle   "o"
          ac_pred     → diamond  "D"
          ac_rajomon  → triangle "^"
        """
        if self.ac == "slack":
            return "D"
        if self.ac == "rajomon":
            return "^"
        return "o"

    @property
    def linestyle(self) -> str:
        """Matplotlib linestyle. Encodes the drop mechanism as a 3-way split so
        abort_slo and abort_slack are visually distinct even when color is similar:
          no drop     → solid     "-"
          abort_slo   → dashed    "--"
          abort_slack → dash-dot  "-."
        """
        if self.drop == "e2e_slo":
            return "--"
        if self.drop == "slack":
            return "-."
        return "-"

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
