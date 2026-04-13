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

**Status:** Rajomon DORMANT ❌ (validation failed — tuning restart required)

| RPS | Target (fifo+ac_rajomon+abort_slo) | Baseline (tailclipper+abort_slo) | Delta |
|-----|------------------------------------|----------------------------------|-------|
| 800 | 799.8 | 799.9 | -0.1 |
| 1200 | 1193.3 | 1189.6 | +3.7 |
| 1400 | **1381.9** | 1179.2 | +202.8 |
| 1600 | 879.3 | 866.1 | +13.2 |
| 1800 | **1161.4** | 991.9 | +169.6 |
| 2000 | 1966.1 | 1826.2 | +139.9 |
| 2500 | 968.6 | 898.8 | +69.8 |
| 3000 | **137.0** | 1050.7 | **-913.7** |

**Rajomon activity: DORMANT.** `abort_reason_timeline.csv` contains only `E2EDeadline` rows for both policies. Zero `RajomonAdmissionRej` / `RajomonChildBudgetRej` events emitted across the whole run. The goodput + ER accounting also holds (goodput + E2EDeadline rate ≈ offered RPS at every load), confirming the admission gate never rejected anything.

**Root cause:** threshold=25ms = half of SLO=50ms is too permissive for socialnet. On hotel (SLO=200ms), 25ms is 1/8 of SLO and queue latency routinely exceeded it. On socialnet, `abort_slo` aborts requests as soon as their 50ms deadline is hit, so queues never get deep enough for queue-latency to exceed 25ms. Price never climbs from init_price=0, accumulated_price stays 0, nothing is rejected at the gate.

**Implication for apparent wins:** The 1400/1800/2000 RPS wins (+170 to +200 goodput) are NOT from rajomon — rajomon was idle. They are artifacts of `sched_fifo,abort_slo` vs `sched_tailclipper,abort_slo` scheduling/abort ordering differences. These wins would survive even if rajomon were removed.

**Implication for 3000 RPS collapse:** The target's collapse to 137 (vs tailclipper's 1050) is also unrelated to rajomon (since rajomon wasn't active). It's a sched_fifo + abort_slo interaction at deep overload. Tailclipper's oldest-first policy fares better at 3000 RPS, but fifo drowns.

**Decision:** Next iteration MUST make rajomon active before any other tuning matters. Lowering `latency_threshold_us` is the single variable to change — it gates every downstream mechanism.

---

## Iteration 2: Lower threshold to activate rajomon (experiment thicket_2)

**Status:** Pending

### Change
Config-only. Single-variable from thicket_1:
- `latency_threshold_us`: 25343 → **5000** (5ms = 1/10 of SLO, mirroring the 1/8 ratio used on hotel)
- All other params unchanged (price_step_up=1, price_cap=500, max_token=5000, price_freq=3, price_update_rate_ms=25)

### Hypothesis
The dormancy in thicket_1 is caused by queue latency never exceeding the 25ms threshold under socialnet's tight 50ms SLO. `abort_slo` drains queues before they grow deep. Lowering threshold to 5ms should let queue-latency excess drive price climb well before SLO-miss rejection kicks in, putting rajomon in the critical path.

### Expected outcomes
1. **Primary (validation):** abort_reason_timeline shows non-zero `RajomonAdmissionRej` at RPS ≥ 1400, and it becomes the dominant reason at some RPS (likely 2500-3000). If this fails, we try an even lower threshold (2000us) in iter 3.
2. Low load (800, 1200) unchanged — queue latency still below 5ms there.
3. 3000 RPS: rajomon activates, gates ingress before runaway abort_slo cascades. Goodput recovers from 137 toward 400+.
4. Possible downside: rajomon may over-reject at moderate overload (1600-2000), reducing goodput vs thicket_1. We accept this as a necessary test of activation; iter 3 will calibrate.

### Experiment design
Identical gen_config and policies as thicket_1. Only `policy_param.json` changes. Enables clean A/B vs thicket_1 for isolating the threshold effect.

### Actual Outcomes (thicket_2)

**Status:** _pending run_
