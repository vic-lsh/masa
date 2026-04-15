from exp_runner.runner.policy import Policy


# ── parse ──────────────────────────────────────────────────────────────────


class TestParse:
    def test_fifo(self):
        p = Policy.parse("sched_fifo")
        assert p.prio == "fifo"
        assert p.est is None
        assert p.drop is None
        assert p.ac is None

    def test_e2e_slo(self):
        p = Policy.parse("sched_slo")
        assert p.prio == "e2e_slo"

    def test_oldest(self):
        p = Policy.parse("sched_tailclipper")
        assert p.prio == "oldest"

    def test_slack(self):
        p = Policy.parse("sched_pred")
        assert p.prio == "slack"
        assert p.est == "mean_var"  # default estimator

    def test_slack_with_explicit_est(self):
        p = Policy.parse("sched_pred,est_mean_var")
        assert p.prio == "slack"
        assert p.est == "mean_var"

    def test_abort_slo(self):
        p = Policy.parse("sched_fifo,abort_slo")
        assert p.drop == "e2e_slo"

    def test_ac_pred(self):
        p = Policy.parse("sched_slo,ac_pred,est_mean_var")
        assert p.ac == "slack"
        assert p.est == "mean_var"  # est shown because ac=slack

    def test_ac_rajomon(self):
        p = Policy.parse("sched_slo,ac_rajomon")
        assert p.ac == "rajomon"
        assert p.est is None  # no slack → no est

    def test_sched_slo_suppressed_by_sched_pred(self):
        """sched_pred implies sched_slo; sched_pred wins."""
        p = Policy.parse("sched_slo,sched_pred,est_mean_var")
        assert p.prio == "slack"

    def test_sched_slo_suppressed_by_tailclipper(self):
        p = Policy.parse("sched_slo,sched_tailclipper")
        assert p.prio == "oldest"

    def test_unknown_policy(self):
        p = Policy.parse("custom_thing")
        assert p.prio is None
        assert p.raw == "custom_thing"

    def test_est_default_for_ac_slack(self):
        """ac=slack without explicit est → default rms."""
        p = Policy.parse("sched_slo,ac_pred")
        assert p.ac == "slack"
        assert p.est == "mean_var"


# ── order agnosticism ──────────────────────────────────────────────────────


class TestOrderAgnostic:
    def test_reversed_flags(self):
        a = Policy.parse("sched_fifo,abort_slo")
        b = Policy.parse("abort_slo,sched_fifo")
        assert a.display_name == b.display_name

    def test_three_flags_any_order(self):
        a = Policy.parse("sched_pred,abort_slo,est_mean_var")
        b = Policy.parse("est_mean_var,sched_pred,abort_slo")
        c = Policy.parse("abort_slo,est_mean_var,sched_pred")
        assert a.display_name == b.display_name == c.display_name

    def test_spaces_around_commas(self):
        a = Policy.parse("sched_fifo, abort_slo")
        b = Policy.parse("sched_fifo,abort_slo")
        assert a.display_name == b.display_name


# ── display_name ───────────────────────────────────────────────────────────


class TestDisplayName:
    def test_fifo_bare(self):
        assert (
            Policy.parse("sched_fifo").display_name == "prio=fifo, drop=none, ac=none"
        )

    def test_fifo_with_drop(self):
        assert (
            Policy.parse("sched_fifo,abort_slo").display_name
            == "prio=fifo, drop=e2e_slo, ac=none"
        )

    def test_fifo_with_ac(self):
        assert (
            Policy.parse("sched_fifo,ac_rajomon").display_name
            == "prio=fifo, drop=none, ac=rajomon"
        )

    def test_fifo_with_drop_and_ac(self):
        assert (
            Policy.parse("sched_fifo,abort_slo,ac_rajomon").display_name
            == "prio=fifo, drop=e2e_slo, ac=rajomon"
        )

    def test_e2e_slo_bare(self):
        assert (
            Policy.parse("sched_slo").display_name == "prio=e2e_slo, drop=none, ac=none"
        )

    def test_e2e_slo_with_drop(self):
        assert (
            Policy.parse("sched_slo,abort_slo").display_name
            == "prio=e2e_slo, drop=e2e_slo, ac=none"
        )

    def test_oldest_bare(self):
        assert (
            Policy.parse("sched_tailclipper").display_name
            == "prio=oldest, drop=none, ac=none"
        )

    def test_oldest_with_drop(self):
        assert (
            Policy.parse("sched_tailclipper,abort_slo").display_name
            == "prio=oldest, drop=e2e_slo, ac=none"
        )

    def test_slack_default_est(self):
        assert (
            Policy.parse("sched_pred").display_name
            == "prio=slack, drop=none, ac=none, est=mean_var"
        )

    def test_slack_explicit_est(self):
        assert (
            Policy.parse("sched_pred,est_mean_var").display_name
            == "prio=slack, drop=none, ac=none, est=mean_var"
        )

    def test_slack_full(self):
        assert (
            Policy.parse("sched_pred,abort_slo,ac_pred,est_mean_var").display_name
            == "prio=slack, drop=e2e_slo, ac=slack, est=mean_var"
        )

    def test_e2e_slo_with_ac_slack(self):
        """ac=slack on a non-slack prio still shows est."""
        assert (
            Policy.parse("sched_slo,ac_pred,est_mean_var").display_name
            == "prio=e2e_slo, drop=none, ac=slack, est=mean_var"
        )

    def test_unknown_fallback(self):
        assert Policy.parse("custom").display_name == "custom"


# ── color ──────────────────────────────────────────────────────────────────


class TestColor:
    """Color encodes (prio, drop) so any two policies differing in scheduling
    priority or abort mechanism get distinct colors.  The AC dimension is
    encoded by marker instead.
    """

    # prio=fifo: grey / sky-blue / blue by drop
    def test_fifo_no_drop(self):
        assert Policy.parse("sched_fifo").color == "#999999"

    def test_fifo_abort_slo(self):
        assert Policy.parse("sched_fifo,abort_slo").color == "#56B4E9"

    def test_fifo_abort_slack(self):
        assert Policy.parse("sched_fifo,abort_slack").color == "#0072B2"

    def test_fifo_abort_slo_ac_pred(self):
        # AC does not change color — marker changes instead.
        assert Policy.parse("sched_fifo,abort_slo,ac_pred").color == "#56B4E9"

    def test_fifo_abort_slack_ac_pred(self):
        assert Policy.parse("sched_fifo,abort_slack,ac_pred").color == "#0072B2"

    # prio=e2e_slo: yellow / black / light-blue by drop
    def test_e2e_slo_no_drop(self):
        assert Policy.parse("sched_slo").color == "#F0E442"

    def test_e2e_slo_abort_slo(self):
        assert Policy.parse("sched_slo,abort_slo").color == "#000000"

    def test_e2e_slo_abort_slo_ac_pred(self):
        assert Policy.parse("sched_slo,abort_slo,ac_pred,est_mean_var").color == "#000000"

    # prio=oldest: dark-pink / reddish-purple / dark-wine by drop
    def test_oldest_no_drop(self):
        assert Policy.parse("sched_tailclipper").color == "#AA3377"

    def test_oldest_abort_slo(self):
        assert Policy.parse("sched_tailclipper,abort_slo").color == "#CC79A7"

    # prio=slack: vermillion / orange / bluish-green by drop
    def test_slack_no_drop(self):
        assert Policy.parse("sched_pred,est_mean_var").color == "#D55E00"

    def test_slack_abort_slo(self):
        assert Policy.parse("sched_pred,abort_slo,est_mean_var").color == "#E69F00"

    def test_slack_abort_slack(self):
        assert Policy.parse("sched_pred,abort_slack,est_mean_var").color == "#009E73"

    def test_slack_abort_slack_ac_pred(self):
        # AC does not change color.
        assert Policy.parse("sched_pred,abort_slack,ac_pred,est_mean_var").color == "#009E73"

    # Key invariant: the problematic pair must have different colors
    def test_fifo_abort_slo_vs_abort_slack_differ(self):
        c1 = Policy.parse("sched_fifo,abort_slo,ac_pred").color
        c2 = Policy.parse("sched_fifo,abort_slack,ac_pred").color
        assert c1 != c2, f"abort_slo and abort_slack must not share a color: {c1}"

    def test_unknown(self):
        assert Policy.parse("custom").color is None


# ── marker / linestyle / hatch ────────────────────────────────────────────


class TestMarker:
    """Marker encodes the admission control mechanism.

    no AC     → circle  "o"
    ac_pred   → diamond "D"
    ac_rajomon → triangle "^"
    """

    def test_no_ac(self):
        assert Policy.parse("sched_fifo").marker == "o"
        assert Policy.parse("sched_slo").marker == "o"
        assert Policy.parse("sched_tailclipper").marker == "o"
        assert Policy.parse("sched_pred,est_mean_var").marker == "o"

    def test_ac_pred(self):
        assert Policy.parse("sched_fifo,abort_slo,ac_pred").marker == "D"
        assert Policy.parse("sched_pred,abort_slack,ac_pred,est_mean_var").marker == "D"

    def test_ac_rajomon(self):
        assert Policy.parse("sched_fifo,ac_rajomon").marker == "^"
        assert Policy.parse("sched_pred,ac_rajomon,abort_slo,est_mean_var").marker == "^"

    def test_drop_does_not_change_marker(self):
        # Drop is encoded by color + linestyle, not marker.
        assert Policy.parse("sched_fifo,abort_slo").marker == "o"
        assert Policy.parse("sched_fifo,abort_slack").marker == "o"

    def test_unknown_fallback(self):
        assert Policy.parse("custom").marker == "o"


class TestLinestyle:
    """Linestyle encodes the drop mechanism as a 3-way split."""

    def test_no_drop_is_solid(self):
        assert Policy.parse("sched_fifo").linestyle == "-"
        assert Policy.parse("sched_slo").linestyle == "-"
        assert Policy.parse("sched_pred,est_mean_var").linestyle == "-"

    def test_abort_slo_is_dashed(self):
        assert Policy.parse("sched_fifo,abort_slo").linestyle == "--"
        assert Policy.parse("sched_slo,abort_slo").linestyle == "--"
        assert Policy.parse("sched_pred,abort_slo,est_mean_var").linestyle == "--"

    def test_abort_slack_is_dash_dot(self):
        # abort_slack gets a distinct linestyle from abort_slo.
        assert Policy.parse("sched_pred,abort_slack,est_mean_var").linestyle == "-."
        assert Policy.parse("sched_fifo,abort_slack,ac_pred").linestyle == "-."

    def test_abort_slo_vs_abort_slack_differ(self):
        ls1 = Policy.parse("sched_pred,abort_slo").linestyle
        ls2 = Policy.parse("sched_pred,abort_slack").linestyle
        assert ls1 != ls2

    def test_unknown_fallback(self):
        assert Policy.parse("custom").linestyle == "-"


class TestHatch:
    """Hatch is a redundant encoding of `prio` for bar plots."""

    def test_fifo(self):
        assert Policy.parse("sched_fifo").hatch == ""

    def test_e2e_slo(self):
        assert Policy.parse("sched_slo").hatch == "//"

    def test_oldest(self):
        assert Policy.parse("sched_tailclipper").hatch == "xx"

    def test_slack(self):
        assert Policy.parse("sched_pred,est_mean_var").hatch == ".."

    def test_unknown_fallback(self):
        assert Policy.parse("custom").hatch == ""
