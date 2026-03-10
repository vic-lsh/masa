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
- Run-to-run variance is ~±20 RPS (compare s1467_18 to s1467_21 at 1800: 979.3 vs 957.4).

**ER breakdown highlights at 1800 RPS:**
- est_mean_var ykccIz2fkK (ms-37691): **241/s** vs prio_local,early **186/s** → +55/s extra inner shedding
- est_mean_var y_DKOh-Gts (ms-37691): **17/s** (feedback loop still broken)
- Root ERs: est_mean_var 295/s vs prio_local,early 264/s (+31 root ERs but still better goodput)

---

## Iteration 2: Add est_child to early-return check (experiment s1467_22)

**Status:** Reverted ❌

**Commit:** 38f2c03d (reverted in c146217a)

### Change

In `before_child_rpc`, added `est_child_mean` (mean client-side child call duration) to the
early-return check. Previously checked `est_remaining_mean > time_left` (not enough time after
child returns). New check: `est_remaining_mean + est_child_mean > time_left` (not enough time
for child call + remaining work).

### Actual Outcomes (s1467_22)

| RPS  | est_mv (s22) | est_mv (s21) | delta abs | diff vs ple (s22) | diff vs ple (s21) |
|------|-------------|-------------|-----------|-------------------|-------------------|
| 1000 | 895.8       | 897.4       | -1.6      | +21.2             | +36.9             |
| 1200 | 910.4       | 925.5       | **-15.1** | +19.6             | +40.3             |
| 1400 | 932.2       | 921.2       | +11.0     | +26.0             | +20.4             |
| 1800 | 983.6       | 957.4       | +26.2     | +25.1             | +24.0             |

**Root cause of 1200 RPS regression:** At moderate overload, `est_child_mean` includes heavy
queueing delay (ms-73106/Sbvx4Hgp0r ERs jumped from 7.7 → 69.3/s; ms-56394/GqI6UW1mU4 ERs
from 1.1 → 42.7/s). The queue-inflated est_child makes the threshold too aggressive for the
actual available capacity, shedding requests that ms-73106 could have served.

The improvement at 1400–1800 RPS confirms that at high overload, the same aggressiveness is
correct (those requests truly can't complete). But the crossover is at ~1300 RPS — below this,
est_child is over-aggressive; above, it's beneficial.

**Key lesson:** Using est_child (client-side duration = queue wait + execution) as a guard can
cause false-positive ERs at moderate overload because the estimate is inflated by queueing that
exists at current load but wouldn't prevent completion at slightly lower service-level utilization.
A pure execution-time estimate (without queue delay) would be needed for safe use.

---

## Iteration 3: Alpha=0.1 (faster EMA adaptation) (experiment s1467_23)

**Status:** Complete ✅ — kept

### Change

In `libs/masa-core/src/latency_estimator/mean_var.rs`, change the default α from 0.05 to 0.1:

```rust
// Before: alpha=0.05, effective window ~20 observations
// After:  alpha=0.1,  effective window ~10 observations
Self::new(1.0, 0.1)
```

### Hypothesis

α=0.05 (20-obs window) was chosen to balance adaptation speed vs noise. Faster α=0.1 (10-obs
window) adapts more quickly when load increases between RPS steps in the sweep. At 1000 RPS, each
edge sees ~1000 obs/s, so the window is ~10ms real time with α=0.05, ~5ms with α=0.1. At 200 RPS,
it's 100ms vs 50ms. The question is whether faster adaptation at the transition points improves
goodput before the estimate settles to the right value.

Also, α=0.1 reduces the impact of stale estimates when load changes. With 0-injections creating a
bimodal distribution, faster adaptation means the EMA recovers more quickly if the ER rate changes.

### Expected outcomes

1. High loads (1400–1800 RPS): neutral to slight improvement — faster adaptation at each new step
2. Moderate loads (1000–1200 RPS): neutral to slight improvement
3. Low loads (800 RPS): no change
4. Risk: more noise in estimate at steady state → slightly higher ER variance → ±2-5 RPS difference
5. Net: within ±10 RPS of s1467_21 at all points if hypothesis is wrong; +5–15 if right

### Actual Outcomes (s1467_23)

**Status: Complete ✅ — kept. Significant improvement at 1800 RPS.**

| RPS  | s23 (α=0.1) | s21 (α=0.05) | delta abs | s23 ple | advantage s23 | advantage s21 |
|------|------------|-------------|-----------|---------|---------------|---------------|
| 200  | 197.0      | 195.0       | +2.0      | 198.6   | -1.7          | -3.6          |
| 400  | 401.9      | 397.5       | +4.4      | 398.1   | +3.8          | +0.3          |
| 800  | 794.6      | 799.8       | -5.2      | 795.7   | -1.1          | -1.7          |
| 1000 | 894.6      | 897.4       | -2.8      | 869.3   | +25.3         | +36.9         |
| 1200 | 919.6      | 925.5       | -5.9      | 879.1   | **+40.5**     | +40.3         |
| 1400 | 929.3      | 921.2       | +8.1      | 891.0   | **+38.3**     | +20.4         |
| 1800 | **993.9**  | 957.4       | **+36.5** | 938.4   | **+55.6**     | +24.0         |

**Key finding:** 1800 RPS goodput = 993.9 — new historical best (vs 979.3 in s1467_18, 957.4 in s21).
Advantage over prio_local,early at 1800 RPS: **+55.6 RPS** (vs +24.0 baseline). Advantage over
prio_oldest,early: **+195.1 RPS**.

All moderate-load changes (-2.8 to -5.9 RPS) are within the ±20 RPS noise band. prio_local,early
itself varied by +8.8 RPS at 1000 between runs (unchanged policy), confirming the noise floor.

**Mechanism:** α=0.1 (10-obs window) adapts twice as fast as α=0.05 (20-obs window). At 1800 RPS
with ~1800 obs/s per edge, the estimate converges in ~5.5ms real time instead of ~11ms. The faster
feedback cycle between ER-induced 0-injections and estimate reduction finds a better equilibrium
for load shedding at near-saturation — shedding earlier and more precisely, freeing child capacity
for requests that can complete.

---

## Iteration 4: Non-monotonic load stress test (experiment s1467_24)

**Status:** Pending

### Change

No code change (α=0.1 from Iteration 3). New experiment config with alternating high/low load:
`[1800, 800, 1800, 800]` — starts cold at 1800 RPS, drops to 800, then repeats the cycle.
The exp_runner saves the last occurrence, so:
- `r1800`: captures t=60–90s (2nd 1800 step, after seeing 1800+800 history)
- `r800`: captures t=90–120s (2nd 800 step, fully adapted to load swings)

### Reference

s1467_14 (same schedule, α=0.05, pre-Iteration 3):
- 1800 RPS: 856.7 goodput
- 800 RPS: 385.8 goodput (stale 1800 estimates caused mass over-shedding)

s1467_17 (schedule [200, 1800, 800], α=0.05, fully fixed EMA):
- 1800 RPS: 976.1 goodput
- 800 RPS: 793.9 goodput

### Hypothesis

α=0.1 adapts to load changes in ~10 observations vs ~20 for α=0.05. At 800 RPS per edge,
10 observations = ~12.5ms real time. The estimate should update within seconds of a load
drop from 1800→800 RPS. The `stale estimates` problem that produced 385.8 at 800 RPS with
α=0.05 in s1467_14 should be significantly reduced — the estimate decays from high-load
values to low-load values twice as fast.

### Expected outcomes

1. 1800 RPS (cold start): ≥ 856.7 (s1467_14 reference); ideally ≥ 950 (close to s23's 993.9)
2. 800 RPS (after 1800 RPS): significantly better than s1467_14's 385.8; ideally ≥ 700
3. Both policies (prio_local,early, prio_oldest,early) serve as internal controls

---

## Open hypotheses (remaining)

**H2 — Alpha tuning (α=0.1):** Faster EMA adaptation (window ≈ 10 obs vs 20) could
improve performance at load transitions between RPS steps. At high RPS, each edge sees
hundreds of observations per second, so α=0.1 is still stable. Risk: increased estimate
noise at steady state. Expected gain: small (+5–15 RPS) if adaptation speed is a bottleneck.

**H3 — Warm prior for cold start:** Initialize the EMA with a reasonable prior (e.g.,
`slo_ms / 2 = 50ms`) instead of starting from 0. Reduces cold-start period.
Expected gain: small for monotonic sweeps; larger for cold-start scenarios.

**H4 — Alpha asymmetric (faster up, slower down):** α_up=0.2 for latency increasing,
α_down=0.05 for latency decreasing. Prevents over-shedding when load drops while reacting
quickly when load increases.

**H5 — Non-monotonic load robustness:** The current performance may be near-optimal for
the monotonic sweep. The bigger opportunity may be ensuring the EMA + 0-injection fix
continues to work well under non-monotonic load (already validated in s1467_17 for
[200, 1800, 800] schedule).
