# THICKET — sched_fifo,ac_rajomon,abort_slo on socialnet ComposePost

## Key questions
- Can rajomon + fifo + abort_slo beat `sched_tailclipper,abort_slo` on socialnet ComposePost across the stress region (≥1400 RPS)?
- Is rajomon actually the dominant rejection mechanism (via `abort_reason_timeline.csv`), or is abort_slo carrying the load while rajomon is effectively dormant? Prior rajomon tuning in this repo didn't validate this — we must.
- What param regime makes rajomon active but non-catastrophic on socialnet's SLO=50ms, shorter-path workload?

## Experiment series: thicket_1 ... thicket_5 (socialnet)

## Validation protocol (every iteration)
After each run, before declaring success or drawing conclusions:
1. Check `exp/socialnet/plots/thicket_N/0/early_return/abort_reason_timeline.csv` (and `.png`).
2. Confirm `RajomonAdmissionRej` (and/or `RajomonChildBudgetRej`) is the dominant abort reason at overloaded RPS points.
3. If abort reason is mostly `SloAbort*` or other non-rajomon reasons at overload, rajomon is dormant — that run is a tuning failure even if goodput looks OK.

## Reference data (prior socialnet rivet runs, no abort-reason validation)

From `exp/socialnet/plots/rivet_5/summary/goodput/goodput_ALL_aggregated.csv`:

| RPS | sched_fifo,ac_rajomon,abort_slo | sched_tailclipper,abort_slo |
|-----|---------------------------------|-----------------------------|
| 800 | 799 | 799 |
| 1000 | 999 | 999 |
| 1200 | 1191 | 1193 |
| 1400 | 1288 | 1351 |
| 1600 | 1052 | 1073 |
| 1800 | 1008 | 983 |
| 2000 | 1087 | 1071 |
| 2500 | 912 | 1091 |
| 3000 | 7.7 | 988 |

Rajomon tracks or slightly beats tailclipper at moderate load (1800-2000) but **collapses at 3000 RPS** (7.7 vs 988). Its abort reason was never validated — the 3000-RPS cliff could be rajomon pathology, or it could be the workload hitting some other wall.

Saturation is ~1400 RPS. Stress region: ≥1400 RPS.

---

## Iteration 1: Establish baseline with hotel-rivet_6 params (experiment thicket_1)

**Status:** Pending

### Change
Config-only. Port hotel-rivet_6's validated rajomon params verbatim to socialnet:
- `latency_threshold_us`: 25343 (25ms — half of socialnet SLO=50ms, so rajomon sees queue-latency pressure before SLO misses)
- `price_update_rate_ms`: 25
- `price_step_up`: 1
- `price_cap`: 500
- `price_freq`: 3
- `tokens_left_init` / `token_update_step` / `max_token`: 5000
- `token_update_rate_ms`: 1

Policies in run: `sched_fifo,ac_rajomon,abort_slo` (target) and `sched_tailclipper,abort_slo` (baseline).

RPS sweep: [800, 1200, 1400, 1600, 1800, 2000, 2500, 3000]. Drop 1000 (uninformative low load) to save runtime; keep the stress region (1400→3000) dense.

### Hypothesis
Hotel-rivet_6 params were the best tuning discovered for rajomon on hotel (SLO=200ms). Socialnet has SLO=50ms and a different call graph (parallel fanout in ComposePost vs hotel's serial Search), so these params may not transfer — but they are principled starting values (threshold at half-SLO, capped price, multi-hop accumulated-price headroom).

### Expected outcomes
1. Abort reason at 1600+ RPS is dominated by `RajomonAdmissionRej` (validation gate).
2. Goodput roughly tracks tailclipper up to 2000 RPS, possibly deviates at 2500-3000.
3. Tells us whether the 3000-RPS cliff from rivet_1/5 was a rajomon pathology or a workload artifact.

### Experiment design
Same gen_config as api_composepost but dropping the uninformative 1000 RPS point. Two policies only (target + tailclipper baseline) to keep the run short and make abort-reason comparison clean.

### Actual Outcomes (thicket_1)

**Status:** _pending run_
