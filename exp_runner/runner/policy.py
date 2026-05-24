"""Parsed representation of a Masa scheduling policy.

A policy is specified as a comma-separated string of Cargo feature flags
(e.g. "sched_pred,abort_slo,est_mean_var"). This module parses that string
into a structured ``Policy`` object with typed fields for each category.
"""

from __future__ import annotations

from dataclasses import dataclass
import hashlib

# Default estimator type when a slack-related flag is present but no explicit
# est_* flag is given.  Must stay in sync with the Rust compile-time default
# in libs/masa-policy/src/layer/est/estimator.rs (LatencyMeanVar fallback).
DEFAULT_EST = "mean_var"

# ── flag → (field, value) mapping ──────────────────────────────────────────

_PRIO_MAP: dict[str, str] = {
    "sched_fifo": "fifo",
    "sched_slo": "e2e_slo",
    "sched_tailclipper": "oldest",
    "sched_oracle": "oracle",
    "sched_pred": "slack",
}

# sched_pred and sched_tailclipper imply sched_slo at the Cargo level.
# When both are present, sched_slo is redundant.
_PRIO_PRIORITY = [
    "sched_pred",
    "sched_tailclipper",
    "sched_oracle",
    "sched_slo",
    "sched_fifo",
]

_EST_MAP: dict[str, str] = {
    "est_mean_var": "mean_var",
    "est_hist": "hist",
    "est_rms": "rms",
}

_DROP_MAP: dict[str, str] = {
    "abort_slack": "slack",
    "abort_slo": "e2e_slo",
    # signal_slack uses the same trigger as abort_slack but does not abort —
    # it only signals admission control. Treated as a third ablation in the
    # same category for plotting purposes.
    "signal_slack": "slack_signal",
}

_AC_MAP: dict[str, str] = {
    "ac_pred": "slack",
    "ac_rajomon": "rajomon",
}

# Flags that do not affect plotting/display metadata.
_IGNORED_FLAGS = {"estimator", "trace_queue_latency"}

_PRIO_DISPLAY: dict[str, str] = {
    "fifo": "FIFO",
    "e2e_slo": "SLO priority",
    "oldest": "TailClipper",
    "oracle": "Oracle priority",
    "slack": "Masa priority",
}

_DROP_DISPLAY: dict[str | None, str] = {
    None: "no drop",
    "e2e_slo": "drop@SLO",
    "slack": "drop@slack",
    "slack_signal": "no drop w/ signal",
}

_FALLBACK_COLORS = [
    "#4E79A7",
    "#F28E2B",
    "#E15759",
    "#76B7B2",
    "#59A14F",
    "#EDC948",
    "#B07AA1",
    "#FF9DA7",
    "#9C755F",
    "#BAB0AC",
]

_FALLBACK_MARKERS = ["o", "s", "D", "^", "v", "P", "X", "<", ">", "h", "*"]

_MARKER_MAP: dict[tuple[str | None, str | None], str] = {
    ("fifo", None): "o",
    ("e2e_slo", None): "s",
    ("oldest", None): "X",
    ("slack", None): "P",
    ("fifo", "slack"): "D",
    ("e2e_slo", "slack"): "d",
    ("oldest", "slack"): "p",
    ("slack", "slack"): "H",
    ("fifo", "rajomon"): "^",
    ("e2e_slo", "rajomon"): "v",
    ("oldest", "rajomon"): "<",
    ("oracle", "rajomon"): "*",
    ("slack", "rajomon"): ">",
}


def _stable_index(raw: str, size: int) -> int:
    digest = hashlib.blake2b(raw.encode("utf-8"), digest_size=8).digest()
    return int.from_bytes(digest, "big") % size


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

    @staticmethod
    def _format_display_name(base: str, details: list[str]) -> str:
        if not details:
            return base
        return f"{base} ({', '.join(details)})"

    @property
    def display_name(self) -> str:
        """Human-readable policy label for plots.

        The canonical Masa policy is ``sched_pred,ac_pred,abort_slack`` and is
        rendered as ``Masa``. Variants render as ``Masa (...)`` with only the
        deviations from that baseline. Rajomon policies use the same short-form
        naming, while estimator flags are intentionally omitted from the label.
        """
        if self.prio is None:
            return self.raw

        if self.ac == "slack":
            deviations: list[str] = []
            if self.prio != "slack":
                deviations.append(_PRIO_DISPLAY[self.prio])
            if self.drop != "slack":
                deviations.append(_DROP_DISPLAY[self.drop])
            return self._format_display_name("Masa", deviations)

        if self.ac == "rajomon":
            if self.prio == "oldest":
                base = "Rajomon"
                details = ["TailClipper"]
            else:
                base = "Rajomon"
                details = ["FIFO" if self.prio == "fifo" else _PRIO_DISPLAY[self.prio]]
            if self.drop is not None:
                details.append(_DROP_DISPLAY[self.drop])
            return self._format_display_name(base, details)

        base = _PRIO_DISPLAY[self.prio]
        details = []
        if self.drop is not None:
            details.append(_DROP_DISPLAY[self.drop])
        return self._format_display_name(base, details)

    @property
    def color(self) -> str | None:
        """Matplotlib color for this policy.

        Encodes (prio, drop) so that any two policies differing in scheduling
        priority or abort mechanism get distinct colors.  The AC dimension is
        encoded by `marker` instead. Unrecognized policies fall back to a
        stable palette indexed by the raw policy string.

        Palette (Okabe-Ito color-blind-safe + grey):

          prio=fifo:
            no drop       → grey          #999999
            abort_slo     → sky blue      #56B4E9
            abort_slack   → blue          #0072B2
            signal_slack  → teal          #44AA99

          prio=e2e_slo:
            no drop       → yellow        #F0E442
            abort_slo     → black         #000000
            abort_slack   → light blue    #88CCEE  (rare)
            signal_slack  → indigo        #332288  (rare)

          prio=oldest (tailclipper):
            no drop       → dark grey     #555555  (groups with FIFO grey,
                                                   used by Rajomon TailClipper)
            abort_slo     → reddish purple #CC79A7
            abort_slack   → dark wine     #882255  (rare)
            signal_slack  → mauve         #DDCC77  (rare)

          prio=slack (pred):
            no drop       → vermillion    #D55E00
            abort_slo     → orange        #E69F00
            abort_slack   → bluish green  #009E73
            signal_slack  → green         #117733
        """
        _color_map: dict[tuple[str | None, str | None], str] = {
            ("fifo", None): "#999999",
            ("fifo", "e2e_slo"): "#56B4E9",
            ("fifo", "slack"): "#0072B2",
            ("fifo", "slack_signal"): "#44AA99",
            ("e2e_slo", None): "#F0E442",
            ("e2e_slo", "e2e_slo"): "#000000",
            ("e2e_slo", "slack"): "#88CCEE",
            ("e2e_slo", "slack_signal"): "#332288",
            ("oldest", None): "#555555",
            ("oldest", "e2e_slo"): "#CC79A7",
            ("oldest", "slack"): "#882255",
            ("oldest", "slack_signal"): "#DDCC77",
            ("oracle", None): "#B07AA1",
            ("oracle", "e2e_slo"): "#AA4499",
            ("oracle", "slack"): "#CC6677",
            ("oracle", "slack_signal"): "#AA3377",
            ("slack", None): "#D55E00",
            ("slack", "e2e_slo"): "#E69F00",
            ("slack", "slack"): "#009E73",
            ("slack", "slack_signal"): "#117733",
        }
        color = _color_map.get((self.prio, self.drop))
        if color is not None:
            return color
        return _FALLBACK_COLORS[_stable_index(self.raw, len(_FALLBACK_COLORS))]

    @property
    def marker(self) -> str:
        """Matplotlib marker shape for this policy.

        Encodes the scheduler + admission-control family so policies that share
        the same color still get a distinct point symbol. Unrecognized policies
        fall back to a stable marker indexed by the raw policy string.
        """
        marker = _MARKER_MAP.get((self.prio, self.ac))
        if marker is not None:
            return marker
        return _FALLBACK_MARKERS[_stable_index(self.raw, len(_FALLBACK_MARKERS))]

    @property
    def linestyle(self) -> str:
        """Matplotlib linestyle. Encodes the drop mechanism so ablation
        variants are visually distinct even when color is similar:
          no drop       → solid     "-"
          abort_slo     → dashed    "--"
          abort_slack   → dash-dot  "-."
          signal_slack  → dotted    ":"
        """
        if self.drop == "e2e_slo":
            return "--"
        if self.drop == "slack":
            return "-."
        if self.drop == "slack_signal":
            return ":"
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
        if self.prio == "oracle":
            return "\\\\"
        if self.prio == "slack":
            return ".."
        return ""
