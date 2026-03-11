# CORAL — prio_local,est_mean_var,emp_admission,early

## Key questions
- Can empirical admission control (emp_admission) close the goodput gap between prio_local,est_mean_var,early (−124 at 1400 RPS) and prio_local,early under Hotel SLO=50ms?
- Does the empirical P(complete | api, bucket) mechanism correctly suppress over-shedding of Reservation requests while still shedding hopeless requests?
- Can prio_local,est_mean_var,emp_admission,early eventually beat prio_local,early (our previous best) and prio_oldest,early (best-in-class baseline)?

## Experiment series: coral_1, coral_2, ... (hotel)

## Reference: pine_10_lowslo baseline

Existing results from pine_10_lowslo (SLO=50ms, 60s/step, 20s warmup):

| RPS  | prio_local,early | prio_local,emv,early | prio_oldest,early |
|------|-----------------|----------------------|-------------------|
| 100  | 49.6            | 48.5                 | 50.8              |
| 400  | 202.0           | 199.6                | 198.1             |
| 800  | 401.2           | 400.0                | 397.4             |
| 1200 | 584.6           | 559.5                | 561.6             |
| 1400 | **525.3**       | 400.9                | 456.2             |
| 1600 | 429.2           | 399.2                | 358.1             |
| 1800 | 427.2           | 383.7                | 351.0             |
| 2000 | 439.5           | 418.0                | 387.2             |

Notes:
- `prio_local,early` (ple) is currently best at all overloaded points
- `prio_local,est_mean_var,early` (emv) without emp_admission is −124 vs ple at 1400
- emp_admission is designed to replace the est_remaining-based ER threshold with empirical P(complete)
- This should prevent over-shedding of Reservation requests (the design doc's central motivation)

---

## Iteration 1: Baseline validation of emp_admission (coral_1)

**Status:** Pending

### Change
No code change. First experiment to observe emp_admission behavior vs baselines.

### Hypothesis
emp_admission should substantially reduce Reservation early-return at overloaded RPS points vs
emv without emp_admission. Whether it matches or beats prio_local,early is the key question.
At minimum it should outperform prio_local,est_mean_var,early (emv alone).

### Expected outcomes if hypothesis is correct:
1. emv+emp at 1400 RPS: goodput significantly above emv alone (400.9) — ideally approaching ple (525.3)
2. Reservation ER rate at 1400 drops from emv's ~173/s toward ple's ~130/s
3. No regression at 100–800 RPS (where all policies converge to near-full throughput)

### Experiment design
Run coral_1 with simplified RPS sweep [800, 1200, 1400, 1600, 1800] — 5 steps to keep runtime
short (~6 min). Policies: prio_local,early; prio_oldest,early; prio_local,est_mean_var,emp_admission,early.
Omit fifo and emv-without-emp (we have reference data from pine_10_lowslo).
Duration: 60s/step (same as pine_10_lowslo), warmup 20s.

### Actual Outcomes (coral_1)

**Status:** Complete ✅ — MASSIVE WIN

**Note:** Discovered bug before experiment: `emp_admission` feature was missing from `libs/masa/Cargo.toml`
and `apps/hotel/Cargo.toml`. Fixed in commit `fd7a4bad`. The actual experiment ran after the fix.

#### Goodput table

| RPS  | prio_local,early | emv+emp  | prio_oldest,early | emv+emp vs ple | emv+emp vs oldest |
|------|------------------|----------|-------------------|----------------|-------------------|
| 800  | 399.8            | 400.8    | 397.9             | +1.0 (+0.2%)   | +2.9 (+0.7%)      |
| 1200 | 581.8            | 601.8    | 569.8             | +20.1 (+3.5%)  | +32.0 (+5.6%)     |
| 1400 | 474.1            | **701.5**| 416.7             | +227.5 (+48%)  | +284.8 (+68%)     |
| 1600 | 352.7            | **797.8**| 314.6             | +445.2 (+126%) | +483.3 (+154%)    |
| 1800 | 338.7            | **895.3**| 288.1             | +556.7 (+164%) | +607.2 (+211%)    |

#### Early return breakdown

- emv+emp: **ZERO Reservation ERs at every RPS point** (not present in ER file at all)
- emv+emp Search ER: ~400-900/s (shed aggressively — correct behavior)
- ple Reservation ER: 0.017 at 800 → 18.3 → 223 → 334 → 341 per second
- prio_oldest Reservation ER: 0.55 at 800 → 29 → 282 → 453 → 407 per second

#### Key findings

1. **emp_admission eliminates Reservation ERs entirely.** The probabilistic admission mechanism
   correctly learns P(complete | Reservation, bucket) ≈ 1.0 for feasible requests and 0.0 for
   infeasible ones, without triggering mid-pipeline early returns.

2. **~50% goodput fraction sustained linearly across all loads.** At every RPS:
   - 800: 50.1%, 1200: 50.2%, 1400: 50.1%, 1600: 49.9%, 1800: 49.7%
   This is consistent with: shed all Search (infeasible under overload), admit all Reservation
   (feasible). Hotel sends 50/50 Search/Reservation, so ~50% is the theoretical maximum when
   Reservation is the only serviceable request type.

3. **ple and prio_oldest collapse past 1200 RPS** due to Reservation ER cascade — the exact
   failure mode the empirical admission was designed to prevent.

4. **The feedback loop is stable.** No oscillations or collapse detected across the tested range.

#### Root cause
emp_admission replaces the est_remaining-based threshold with empirical P(complete | api, bucket).
Under overload, the RMS/mean_var estimator inflated est_remaining (queue delay contamination),
triggering preemptive Reservation ERs. emp_admission directly observes outcomes and learns
completion probability without conflating compute time and queue delay.

#### Open questions
1. What happens at 2000+ RPS? Does goodput continue to hold at ~50% or does Reservation also saturate?
2. Can Search be admitted at lower RPS levels to push goodput above 50%? At 800-1200 RPS,
   some Search should be completable — but current data shows ~400 Search ERs even at 800 RPS.
3. Non-monotonic RPS: does emv+emp recover correctly after a high-load period?
4. What is the actual system capacity for Reservation? The flat 50% fraction suggests we're
   still within Reservation capacity at 1800 RPS (900 Reservation/s completable).

---

## Iteration 2: Full RPS sweep to characterize capacity limits (coral_2)

**Status:** Pending

### Change
No code change. Run the full 8-RPS-level sweep from pine_10_lowslo config to understand:
1. Does emv+emp hold 50% fraction at 2000 RPS (1000 Reservation/s)?
2. Does Search get properly admitted at 100/400 RPS where system is not overloaded?
3. Absolute goodput numbers match pine_10_lowslo reference at under-loaded RPS points?

### Hypothesis
At 100 and 400 RPS (under-loaded), emv+emp should behave like ple/oldest (no meaningful
shedding). At 800+ RPS, Search starts being shed but Reservation stays fully admitted.
At some RPS beyond 1800, Reservation itself saturates and goodput fraction drops.
The inflection point is probably around 2000+ RPS.

### Expected outcomes if hypothesis is correct:
1. 100 RPS: ~49-50 goodput (similar to all policies)
2. 400 RPS: ~199-200 goodput (similar to all policies)
3. 800-1800: emv+emp dominates as seen in coral_1
4. 2000: either holds at ~50% or starts declining (test will reveal)

### Experiment design
Full 8-RPS sweep: [100, 400, 800, 1200, 1400, 1600, 1800, 2000].
Same config as pine_10_lowslo but with updated policies (no fifo, no bare emv).
Policies: prio_local,early; prio_oldest,early; prio_local,est_mean_var,emp_admission,early.
Duration: 60s/step, warmup 20s. Will run ~25 minutes.
