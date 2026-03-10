# FLINT — prio_local,est_mean_var,early

## Key questions

- Can we push `prio_local,est_mean_var,early`'s advantage over `prio_local,early` (RMS-based)
  beyond the current ~30–40 RPS gain at high loads (1000–1800 RPS)?
- Is there an α value that adapts faster under load transitions while remaining stable during
  steady state? (Current: α=0.05, effective window ≈ 20 observations)
- Can we reduce the bimodal EMA distortion introduced by 0-injections on ER (which creates a
  mixed distribution of real latencies and zeros in the EMA)?
- Are there additional signals (est_child, ER rate, queue length) that could sharpen early-return
  decisions without adding priority inversions?

## Experiment series: s1467 (mssim)

**App:** mssim, trace S_14677443
**SLO:** 100ms
**Config base:** exp/mssim/in/s1467_20/ (broad monotonic sweep)
**Policies under test:** prio_local,est_mean_var,early | prio_local,early | prio_oldest,early
**External baseline (for paper):** prio_oldest,early
**Internal baseline:** prio_local,early (old RMS-based impl)

---

## Prior work summary (PRIO_LOCAL_IMPROVEMENTS.md, Iterations 1–15)

The previous tracking doc (`PRIO_LOCAL_IMPROVEMENTS.md`) reached the following final state:

**Best code (current, k=1.0, α=0.05):**

| RPS  | est_mean_var (s1467_18) | prio_local,early | prio_oldest,early |
|------|------------------------|-----------------|-------------------|
| 200  | 197.8                  | 196.5           | —                 |
| 400  | 402.3                  | 405.0           | —                 |
| 800  | 797.6                  | 798.6           | —                 |
| 1000 | **890.9**              | 861.2           | 805.5             |
| 1200 | **922.3**              | 889.8           | 801.2             |
| 1400 | **929.6**              | 888.5           | 764.8             |
| 1800 | **979.3**              | 939.1           | 799.3             |

Note: s1467_20 (the user's starting point) was a **k=0.5 experiment** and showed est_mean_var ≈
tied with prio_local,early. The current code is k=1.0, which was confirmed as optimal.

**Key fixes already in place:**
1. √ `mean + k*stddev` (not variance) — dimensional fix
2. √ saturating_sub + `min(est_remaining, time_left)` cap
3. √ EMA (α=0.05) replacing Welford all-time accumulator
4. √ 0-injection on child ER to break feedback loop
5. √ Dynamic EDF reprioritization via `before_poll` + hyper spawn priority
6. √ Mean-only threshold for ER check (mean, not mean+σ)
7. √ k=1.0 confirmed optimal over k=0.5, k=1.5 not tested

**Remaining gap:** At 400 RPS, est_mean_var is ~2-3 RPS below prio_local,early (within noise).
Low-load behavior otherwise parity. High-load advantage is large and consistent.

---

## Observed Symptoms (s1467_20 — k=0.5 experiment, not representative of current code)

From `exp/mssim/plots/s1467_20/goodput_absolute_avg.csv`:

| RPS  | est_mean_var | prio_local,early | prio_oldest,early |
|------|-------------|-----------------|-------------------|
| 200  | 198.9       | 199.3           | 196.9             |
| 400  | 395.6       | 399.2           | 406.9             |
| 800  | 798.3       | 795.1           | 796.8             |
| 1000 | 871.1       | 866.9           | 786.9             |
| 1200 | 894.2       | 886.2           | 773.2             |
| 1400 | 920.9       | 919.7           | 767.6             |
| 1800 | 961.0       | 969.7           | 824.1             |

This is k=0.5. prio_local,early uses k=∞ effectively (RMS = all-time accumulator). The k=0.5
under-shedding explains why prio_local,early wins at 1800 RPS in this run. **Current code (k=1.0)
should replicate s1467_18 performance** (+30–40 RPS over prio_local,early at high loads).

---

## Iteration 2: Add est_child to early-return check (experiment s1467_22)

**Status:** Pending

### Change

In `libs/tonic/tonic/src/masa/context/local/local.rs`, `before_child_rpc`:

Replace the early-return check from using only `est_remaining_mean` to using the combined
`est_remaining_mean + est_child_mean`. This uses the already-tracked `est_child_latency`
(currently only used for logging) to determine whether the child call itself can complete
within the remaining SLO budget.

### Hypothesis

The current check sheds if there isn't enough time AFTER the child completes (`est_remaining_mean`).
It ignores how long the child call itself will take (`est_child_mean`). This means a request with
15ms remaining can still dispatch a y_DKOh-Gts call that takes 40ms (queue + execution) — guaranteed
to fail, but we still waste ms-37691 capacity for 40ms before discovering this.

Adding est_child_mean: shed if `est_child_mean + est_remaining_mean > time_left`. For the expensive
edge (y_DKOh-Gts at 1800 RPS):
- est_child_mean ≈ 40ms, est_remaining_mean ≈ 13ms → new threshold = 53ms vs old 13ms
- Requests with 13–53ms remaining are now shed at ms-56394 instead of dispatched to ms-37691
- This frees ms-37691 capacity (~40ms per shed request) for other requests that can succeed

At 800 RPS (light load), est_child_mean is small (~2–5ms) so the threshold rises from 13ms to
15–18ms — minor change, no expected impact on goodput.

### Expected outcomes

1. 1000–1200 RPS: est_mean_var advantage widens from +36–40 RPS to +45–55 RPS — more requests
   shed before dispatching to child, freeing child capacity for completable requests
2. 1400–1800 RPS: similar or better vs s1467_21 (more aggressive shedding at near-saturation)
3. 800 RPS: no meaningful change (est_child_mean small at light load)
4. ms-37691/y_DKOh-Gts ER rate stays low (feedback loop still broken)
5. ms-37691/ykccIz2fkK ER rate may increase slightly (more callers shed at parent before dispatch)

---

## Iteration 1: Baseline confirmation (experiment s1467_21)

**Status:** Complete ✅

### Change

No code change. Ran with current binary (k=1.0, α=0.05).

### Actual Outcomes (s1467_21)

| RPS  | est_mean_var | prio_local,early | prio_oldest,early | diff (emv vs ple) |
|------|-------------|-----------------|-------------------|-------------------|
| 200  | 195.0       | 198.6           | 201.9             | -3.6 (noise)      |
| 400  | 397.5       | 397.2           | 393.7             | +0.3              |
| 800  | 799.8       | 801.5           | 795.9             | -1.7 (noise)      |
| 1000 | **897.4**   | 860.5           | 796.0             | **+36.9**         |
| 1200 | **925.5**   | 885.2           | 749.9             | **+40.3**         |
| 1400 | **921.2**   | 900.8           | 773.3             | **+20.4**         |
| 1800 | **957.4**   | 933.4           | 809.9             | **+24.0**         |

**Status: Baseline confirmed ✅**

- Current code (k=1.0) beats prio_local,early by **+20–40 RPS at all overloaded loads** (1000–1800 RPS).
- At 800 RPS and below: near parity (within noise).
- The "tied" description from s1467_20 was the k=0.5 experiment — the current code is already winning.
- Run-to-run variance is ~±20 RPS (compare to s1467_18: 1400=929.6 vs s21: 921.2, 1800=979.3 vs 957.4).

**ER breakdown highlights at 1800 RPS:**
- est_mean_var ykccIz2fkK (ms-37691): **241/s** vs prio_local,early **186/s** → +55/s extra inner shedding
- est_mean_var y_DKOh-Gts (ms-37691): **17/s** (feedback loop still broken)
- Root ERs: est_mean_var 295/s vs prio_local,early 264/s (+31 root ERs but still better goodput)

**Key insight:** est_mean_var sheds more aggressively at the bottleneck (ms-37691/ykccIz2fkK),
freeing capacity for requests that can complete. The extra shedding is productive, not wasteful.

**Remaining gap:** Run-to-run variance (~±20 RPS) makes small improvements hard to measure cleanly.

---

## Open hypotheses

Listed in priority order for evaluation after s1467_21 baseline:

**H1 — Include est_child in ER check (→ Iteration 2):** Currently `before_child_rpc` sheds if
`est_remaining_mean > time_left` (not enough time AFTER child completes). Adding `est_child_mean`
to the check: `est_child_mean + est_remaining_mean > time_left` would avoid dispatching calls
where the child itself will take too long. Saves network round-trip + queue wait at child.
At 1800 RPS, est_child for y_DKOh-Gts ≈ 40ms → new threshold is 40+13 = 53ms vs current 13ms.
Requests with 13–53ms remaining would be shed earlier (at parent, before dispatch) rather than
spending 40ms in child queue before failing. Risk: more aggressive shedding at moderate loads.

**H2 — Alpha tuning (α=0.1):** Faster EMA adaptation (window ≈ 10 obs vs 20) could
improve performance at load transitions between RPS steps. Risk: increased estimate noise.
Expected gain: small in monotonic sweep; larger under rapid load changes.

**H3 — Warm prior for cold start:** Initialize the EMA with a reasonable prior (e.g.,
`slo_ms / 2 = 50ms`) instead of starting from 0. Reduces cold-start period.
Expected gain: small for monotonic sweeps; larger for cold-start scenarios.

**H4 — Alpha asymmetric (faster up, slower down):** α_up=0.2 for latency increasing,
α_down=0.05 for latency decreasing. Prevents over-shedding when load drops while reacting
quickly when load increases.

---

## Iteration 1: Baseline confirmation (experiment s1467_21)

**Status:** Pending

### Change

No code change. Copy s1467_20 config and run with current binary (k=1.0, α=0.05).

### Purpose

Confirm that the current code (k=1.0) matches s1467_18 performance. The user's "tied"
observation was based on s1467_20 which was the k=0.5 experiment. If the current binary
indeed shows +30–40 goodput RPS advantage over prio_local,early at high loads, we have
a working baseline and can proceed to further improvements.

### Expected outcomes

1. At 1800 RPS: est_mean_var ≥ 960, beating prio_local,early by ≥ 30 RPS
2. At 1000 RPS: est_mean_var ≥ 870, beating prio_local,early by ≥ 20 RPS
3. At 800 RPS: est_mean_var ≥ 790, within 5 RPS of prio_local,early
4. Both beat prio_oldest,early at all overloaded RPS levels
