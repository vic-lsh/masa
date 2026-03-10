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
