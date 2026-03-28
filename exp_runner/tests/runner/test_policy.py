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
        assert p.est == "rms"  # default estimator

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
        assert p.est == "rms"


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
            == "prio=slack, drop=none, ac=none, est=rms"
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
    def test_fifo(self):
        assert Policy.parse("sched_fifo").color == "grey"

    def test_fifo_drop(self):
        assert Policy.parse("sched_fifo,abort_slo").color == "darkgrey"

    def test_e2e_slo(self):
        assert Policy.parse("sched_slo").color == "steelblue"

    def test_e2e_slo_drop(self):
        assert Policy.parse("sched_slo,abort_slo").color == "cornflowerblue"

    def test_oldest(self):
        assert Policy.parse("sched_tailclipper").color == "purple"

    def test_oldest_drop(self):
        assert Policy.parse("sched_tailclipper,abort_slo").color == "mediumpurple"

    def test_slack_with_est(self):
        assert Policy.parse("sched_pred,est_mean_var").color == "coral"

    def test_slack_with_ac(self):
        assert Policy.parse("sched_pred,ac_pred,est_mean_var").color == "forestgreen"

    def test_unknown(self):
        assert Policy.parse("custom").color is None
