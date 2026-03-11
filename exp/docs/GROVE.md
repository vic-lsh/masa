# GROVE — prio_local,est_mean_var,early

## Key questions
- Why does `prio_local,est_mean_var,early` underperform `prio_local,early` near saturation (1400 RPS) when SLO is tight (50ms), and how can we fix it?
- Can EMA-based latency estimation be made as overload-resistant as the RMS estimator's accidental batch-update lag, while retaining EMA's adaptation advantages?
- Can emv match or exceed prio_local,early across the full RPS range while preserving its advantage over prio_oldest at deep overload?

## Experiment series: grove_1, grove_2, ... (hotel)

## Baseline: pine_10_lowslo (Hotel, SLO=50ms)

Config: `exp/hotel/in/pine_10_lowslo/` — SLO=50ms for both Search and Reservation, RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000], WarmupSecs=20, DurationSecs=60.

Code state: k=0, α=0.1 (symmetric), e2e_deadline for EarlyReturnHandler and before_child_rpc ER check, before_poll uses ctx.deadline() for EDF reprioritization.

### Observed Symptoms (pine_10_lowslo)

**Goodput table:**

| RPS  | fifo  | prio_local,early | prio_local,emv | prio_oldest,early | emv−ple   | emv−oldest |
|------|-------|-----------------|----------------|-------------------|-----------|------------|
| 100  | 48.6  | 49.6            | 48.5           | 50.8              | −1.1      | −2.3       |
| 400  | 200.5 | 202.0           | 199.6          | 198.1             | −2.4      | +1.5       |
| 800  | 396.6 | 401.2           | 400.0          | 397.4             | −1.2      | +2.5       |
| 1200 | 547.9 | 584.6           | 559.5          | 561.6             | −25.1     | −2.1       |
| 1400 | 163.9 | 525.3           | **400.9**      | **456.2**         | **−124.4**| **−55.4**  |
| 1600 | 163.0 | 429.2           | **399.2**      | **358.1**         | **−30.0** | **+41.1**  |
| 1800 | 180.0 | 427.2           | **383.7**      | **351.0**         | **−43.5** | **+32.7**  |
| 2000 | 203.5 | 439.5           | **418.0**      | **387.2**         | **−21.5** | **+30.7**  |

**Early return rates (Reservation ER/s):**

| RPS  | prio_local,early | prio_local,emv | prio_oldest |
|------|-----------------|----------------|-------------|
| 1200 | 14.1            | 38.6           | 35.2        |
| 1400 | 172.7           | **295.1**      | 238.9       |
| 1600 | 288.8           | 327.3          | 421.9       |
| 1800 | 275.3           | 305.4          | 379.4       |
| 2000 | 271.4           | 354.2          | 391.4       |

**Root cause analysis:**

At 1400 RPS (saturation knee), EMA with α=0.1 (~10-obs effective window) adapts immediately to inflated post-child service times. This inflates `est_remaining_mean`, tightening the ER threshold (`e2e_deadline - est_remaining_mean`) in `before_child_rpc`. Reservation ERs spike to 295/s vs 172/s for prio_local,early and 238/s for prio_oldest — excessive shedding that costs 124 goodput.

`prio_local,early` (RMS estimator) uses a 512-observation update batch, anchoring its estimate to historical low-load values throughout the entire sweep. This is "accidental" overload resistance that EMA α=0.1 doesn't have.

At deep overload (1600–2000 RPS), emv beats prio_oldest (+32 to +41) because EDF priority ordering (`before_poll` using tightened `ctx.deadline()`) improves scheduling. Reservation ER for prio_oldest is actually higher (421/s vs emv's 327/s at 1600), so the EDF benefit overcomes the ER difference.

The 50ms SLO amplifies the problem: any nonzero `est_remaining` has proportionally greater impact than with 200ms SLO.

**Key code locations:**
- `libs/tonic/tonic/src/masa/context/local/local.rs:299` — ER threshold check: `time_now() > e2e_deadline - est_remaining_mean`
- `libs/tonic/tonic/src/masa/context/local/local.rs:350–358` — 0-injection on ER (negative feedback, but insufficient)
- `libs/masa-core/src/latency_estimator/mean_var.rs:55–57` — EMA update: `mean += α*(x - mean)`, fires every observation

**Goal:** Close the 124-goodput gap at 1400 RPS while preserving the +32 to +41 advantage over prio_oldest at 1600–2000.

---

## Iteration 1: Asymmetric α — slow inflation, fast deflation (grove_1)

**Status:** Pending

### Change
In `LatencyMeanVar`, use two α values instead of one:
- `α_up = 0.05` when `x > mean` (new observation is above current estimate) — slower to inflate
- `α_down = 0.2` when `x ≤ mean` (new observation is below current estimate) — faster to deflate

This applies to the EMA update in `mean_var.rs` and also to the variance update (to keep variance consistent).

### Hypothesis
The primary failure mode is EMA mean inflation at the saturation knee: post-child durations spike from queue buildup, EMA α=0.1 tracks them upward within ~10 observations, inflating `est_remaining_mean` → premature ER fires at 295/s. By slowing upward adaptation (α_up=0.05), we resist transient overload spikes. By keeping fast downward adaptation (α_down=0.2), we recover quickly from 0-injections and load decreases.

FLINT Iteration 6 tried the *opposite* asymmetry (α_up=0.2, α_down=0.1) and saw "ratchet-bias" regression — estimates ratcheted up and were slow to come down. Our direction (slow up, fast down) avoids ratchet bias and targets the specific failure mode.

### Expected outcomes if hypothesis is correct:
1. Reservation ER at 1400 RPS drops from 295/s toward prio_local,early's 172/s (target ~200/s)
2. Goodput at 1400 RPS improves from 400.9 toward 525.3 — target: within 30 of prio_local,early
3. No regression at 1600–2000 RPS (where emv currently beats prio_oldest by +32 to +41)
4. Minimal change at ≤1200 RPS (all policies already perform similarly)

### Experiment design
Copy pine_10_lowslo → grove_1 (same RPS sweep, same SLO=50ms). The saturation knee at 1400 RPS is the critical test point. The full sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000] provides regression coverage across all load levels.

### Actual Outcomes (grove_1)

**Status:** Keep ✅ — large improvement, no regressions

**Goodput table:**

| RPS  | emv grove_1 | emv pine10 | ple grove_1 | oldest grove_1 | emv−ple  | Δemv (grove1−pine10) |
|------|-------------|------------|-------------|----------------|----------|----------------------|
| 100  | 49.0        | 48.5       | 49.4        | 50.2           | −0.3     | +0.6                 |
| 400  | 199.6       | 199.6      | 199.3       | 198.3          | +0.3     | −0.0                 |
| 800  | 398.4       | 400.0      | 402.0       | 395.3          | −3.6     | −1.6                 |
| 1200 | **585.0**   | 559.5      | 583.3       | 562.9          | **+1.7** | **+25.5**            |
| 1400 | **471.6**   | 400.9      | 511.3       | 456.4          | **−39.7**| **+70.8**            |
| 1600 | 415.5       | 399.2      | 414.5       | 403.1          | +0.9     | +16.3                |
| 1800 | 388.8       | 383.8      | 416.8       | 372.8          | −27.9    | +5.1                 |
| 2000 | 449.1       | 418.0      | 467.6       | 392.4          | −18.5    | +31.1                |

**Early return rates (Reservation ER/s):**

| RPS  | emv grove_1 | emv pine10 | ple grove_1 | prio_oldest |
|------|-------------|------------|-------------|-------------|
| 1200 | 11.8        | 38.6       | 17.7        | 38.4        |
| 1400 | **223.4**   | 295.1      | 183.9       | 243.5       |
| 1600 | 276.8       | 327.3      | 308.4       | 375.5       |
| 1800 | 278.5       | 305.4      | 292.7       | 350.5       |
| 2000 | 291.9       | 354.2      | 316.1       | 371.3       |

**Key findings:**
1. At 1400 RPS: Reservation ER dropped from 295/s → 223/s (−72/s); goodput improved from 400.9 → 471.6 (+70.8). Recovered ~57% of the 124-point gap to prio_local,early. Remaining gap: −39.7.
2. At 1200 RPS: large surprise win (+25.5) — ER dropped from 38.6 → 11.8/s. emv now ties ple (585 vs 583).
3. emv now beats prio_oldest at ALL overloaded RPS (1400–2000). This is a qualitative fix of the Hotel failure mode.
4. No regressions anywhere. The asymmetric α direction is clearly correct.

**Remaining gap:** At 1400 RPS, emv ER is still 223/s vs ple's 184/s — over-shedding Reservation by ~39/s. This explains the remaining −39.7 goodput gap. Need to further slow inflation.

**Decision: KEEP. Proceed to Iteration 2: lower α_up further (0.02) to close the remaining ER gap at 1400 RPS.**

---

## Iteration 2: Lower α_up to 0.02 (grove_2)

**Status:** Pending

### Change
In `LatencyMeanVar`, lower `α_up` from 0.05 → 0.02 (half-step down). Keep `α_down = 0.2`.

### Hypothesis
At 1400 RPS, emv still over-sheds Reservation (223/s vs ple's 184/s). The estimate still inflates too fast when queue delays spike. By halving α_up from 0.05 to 0.02 (effective window ~50 obs for upward moves), we further slow the inflation of `est_remaining_mean`, pushing the ER threshold closer to the true saturation point. α_down=0.2 ensures rapid deflation from 0-injections and load drops, so there's no risk of stale over-estimates.

### Expected outcomes if hypothesis is correct:
1. Reservation ER at 1400 RPS drops from 223/s toward ple's 184/s (target ~190/s)
2. Goodput at 1400 RPS improves from 471.6 toward ple's 511.3 — target: within 20 of ple
3. No regression at 1600–2000 RPS (emv should maintain its advantage over prio_oldest)
4. Minimal change at ≤1200 RPS (already improved by Iteration 1)

### Experiment design
Same grove_1 config (RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000], SLO=50ms). Named grove_2.
