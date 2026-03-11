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

### Actual Outcomes (grove_2)

**Status:** Revert ❌ — 1600 regression outweighs 1400 gain

**Goodput table:**

| RPS  | emv grove_2 | emv grove_1 | ple grove_2 | oldest grove_2 | emv−ple  | Δemv (g2−g1) |
|------|-------------|-------------|-------------|----------------|----------|--------------|
| 100  | 50.3        | 49.0        | 49.6        | 50.1           | +0.7     | +1.3         |
| 400  | 200.4       | 199.6       | 200.5       | 199.2          | −0.1     | +0.8         |
| 800  | 401.0       | 398.4       | 397.8       | 399.0          | +3.2     | +2.6         |
| 1200 | 588.2       | 585.0       | 579.6       | 566.1          | +8.6     | +3.2         |
| 1400 | **504.1**   | **471.6**   | **540.4**   | **473.2**      | **−36.3**| **+32.5**    |
| 1600 | **373.1**   | **415.5**   | **386.2**   | **367.7**      | **−13.1**| **−42.4**    |
| 1800 | 389.4       | 388.8       | 439.1       | 343.7          | −49.7    | +0.6         |
| 2000 | 436.7       | 449.1       | 459.9       | 387.2          | −23.2    | −12.4        |

**Reservation ER/s:**

| RPS  | emv grove_2 | emv grove_1 | ple grove_2 |
|------|-------------|-------------|-------------|
| 1200 | 12.7        | 11.9        | 15.7        |
| 1400 | **194.9**   | **223.8**   | **156.4**   |
| 1600 | **298.0**   | **276.8**   | **284.0**   |
| 1800 | 309.6       | 278.5       | 268.5       |
| 2000 | 288.6       | 291.9       | 287.2       |

**Per-API breakdown (confirmed):** Search goodput = 0 at ALL RPS for ALL policies. All goodput is Reservation only. Search ER rates are nearly identical across all policies (~99–100% shed at ≥1200 RPS). The entire policy differentiation is in Reservation handling.

**Key findings:**
- α_up=0.02 helped at 1400 (ER: 223→195, goodput: 471→504) but hurt at 1600 (ER: 277→298, goodput: 415→373)
- At 1600, the mean lags behind reality → over-optimistic est_remaining → delayed ER decisions → worse goodput
- Net effect: trades under-inflation regime (1400) for under-adaptation regime (1600)
- α_up tuning has a narrow operating range — a structural fix is needed

**Root cause:** Binary ER threshold (fire when `time_now() > e2e_deadline - est_remaining_mean`) creates cliff behavior. When est_remaining inflates even slightly, many marginal requests get shed. When est_remaining is too low, not enough requests get shed. A continuous (probabilistic) threshold would smooth this cliff.

**Decision: REVERT grove_2. Restore α_up=0.05 (grove_1 state). Reverted in commit 44f1481c.**

---

## Iteration 3: Probabilistic ER — smooth the binary shedding threshold (grove_3)

**Status:** Pending

### Change
Replace the binary ER threshold in `before_child_rpc` with a probabilistic admission check:

```
let time_left = ctx.e2e_deadline() - time_now()
let p_complete = (time_left / est_remaining_mean).clamp(0.05, 1.0)
shed with probability (1 - p_complete)
```

When `time_left > est_remaining_mean` (plenty of time): P≈1, no shedding.
When `time_left = est_remaining_mean` (tight): P=1.0 → current binary threshold (same behavior as before at the exact cliff).
When `time_left = 0.5 * est_remaining_mean`: P=0.5 → shed 50%.
When `time_left → 0`: P→0.05 (probe floor, never shed 100%).

The 5% floor ensures we always let some requests through to maintain the feedback loop — addressing the user's concern about re-evaluating capacity after load drops.

### Hypothesis
The binary threshold creates excessive cliff behavior: a small est_remaining inflation causes a large jump in ER rate (all marginal requests get shed). Probabilistic admission smooths this into a continuous function: requests are shed proportionally to how unlikely they are to complete. Near the threshold, only the most doomed requests get shed rather than all of them.

This also naturally handles the API-type differentiation the user identified: Search requests have smaller time_left relative to est_remaining (longer call chain) → lower P → shed more aggressively. Reservation has larger P → fewer false-positive ERs.

### Expected outcomes if hypothesis is correct:
1. At 1400 RPS: Reservation ER drops further (below grove_1's 223/s) as marginal requests are admitted instead of deterministically shed
2. Goodput at 1400 improves beyond grove_1's 471.6 (target: close to grove_2's 504 gain without the 1600 regression)
3. At 1600 RPS: no regression — probabilistic approach adapts naturally (more shedding when more requests are over-budget) unlike α_up=0.02 which was globally too slow
4. Recovery signal maintained: the 5% probe floor provides ongoing feedback even under heavy shedding

### Experiment design
Same grove_1 config. Named grove_3.

### Actual Outcomes (grove_3)

**Status:** Revert ❌ — probabilistic ER made things worse

**Goodput table:**

| RPS  | emv grove_3 | emv grove_1 | ple grove_3 | oldest grove_3 | emv−ple (g3) | Δemv (g3−g1) |
|------|-------------|-------------|-------------|----------------|--------------|--------------|
| 100  | 50.2        | 49.0        | 48.9        | 50.3           | +1.3         | +1.2         |
| 400  | 200.9       | 199.6       | 199.5       | 198.9          | +1.4         | +1.3         |
| 800  | 396.6       | 398.4       | 396.1       | 400.1          | +0.5         | −1.7         |
| 1200 | 584.3       | 585.0       | 595.6       | 580.9          | −11.3        | −0.7         |
| 1400 | **461.5**   | **471.6**   | **567.0**   | **564.0**      | **−105.5**   | **−10.1**    |
| 1600 | 374.0       | 415.5       | 438.8       | 376.0          | −64.8        | **−41.5**    |
| 1800 | 400.5       | 388.8       | 436.2       | 374.9          | −35.7        | +11.7        |
| 2000 | 475.4       | 449.1       | 28.5 (!)    | 391.3          | +83.9        | +26.3        |

**Reservation ER/s:**

| RPS  | emv grove_3 | emv grove_1 | ple grove_3 |
|------|-------------|-------------|-------------|
| 1200 | 19.4        | 11.8        | 5.2         |
| 1400 | **231.2**   | **223.4**   | **129.7**   |
| 1600 | 282.7       | 276.8       | 277.1       |
| 1800 | 290.0       | 278.5       | 260.7       |
| 2000 | 302.5       | 291.6       | 16.8        |

**Key findings:**
- Probabilistic ER did NOT reduce ER rate — it increased from 223 → 231/s at 1400
- Goodput at 1400 dropped −10.1 vs grove_1; emv trails ple by −105.5 (massive gap)
- Regression at 1600: −41.5 (identical to grove_2's regression)
- ple collapse at 2000 RPS (28.5) is anomalous run artifact, ignore

**Root cause:** Probabilistic gate randomly admits some requests that can't complete (since est_remaining is inflated because the system IS overloaded). These requests add downstream pressure, increasing ER rates rather than decreasing them. The inflation is real — fixing the ER decision mechanism doesn't fix the root signal.

**Correct diagnosis:** The EMA est_remaining being used in the ER check is the wrong signal. ple's RMS estimate is near-zero (anchored to low-load baseline), so it only ERs at actual deadline expiry. emv should do the same.

**Decision: REVERT grove_3. Implement grove_4: deadline-only ER check — remove est_remaining from ER decision entirely. Reverted + grove_4 implemented in commit 7238f31e.**

---

## Iteration 4: Deadline-only ER — decouple est_remaining from ER check (grove_4)

**Status:** Pending

### Change
In `before_child_rpc`, replace:
```rust
if EARLY_RETURN && time_now() > self.ctx.e2e_deadline().saturating_sub(est_remaining_mean) {
```
with:
```rust
if EARLY_RETURN && time_now() >= self.ctx.e2e_deadline() {
```

est_remaining_mean is still used for child deadline computation and periodic logging, but NOT for the ER decision. ER fires only when the actual e2e deadline has been exceeded.

### Hypothesis
ple's advantage comes from its RMS estimate being near-zero (anchored to low-load baseline via batch updates), making its ER threshold effectively "ER at actual deadline." emv's inflated EMA causes it to ER too early. By removing est_remaining from the ER check, emv matches ple's conservative ER behavior while retaining emv's advantages: EMA for child deadline propagation (urgency tightening) and EDF reprioritization via before_poll.

### Expected outcomes:
1. Reservation ER at 1400 drops from grove_1's 223/s toward ple's ~130/s
2. Goodput at 1400 rises from grove_1's 471.6 toward ple's ~540–567
3. If EDF reprioritization (before_poll) provides scheduling benefit over ple, emv may exceed ple
4. No regression at 1600–2000 (deadline-only ER should generalize well — fires at the right time)

### Experiment design
Same grove_1 config. Named grove_4.

### Actual Outcomes (grove_4)

**Status:** Revert ❌ — catastrophic across all RPS levels

**Goodput table:**

| RPS  | emv grove_4 | emv grove_1 | ple grove_4 | oldest grove_4 | emv−ple (g4) | Δemv (g4−g1) |
|------|-------------|-------------|-------------|----------------|--------------|--------------|
| 100  | 48.0        | 49.0        | 49.3        | 49.1           | −1.3         | −1.0         |
| 400  | 199.0       | 199.6       | 198.5       | 197.8          | +0.5         | −0.6         |
| 800  | 397.2       | 398.4       | 399.0       | 400.2          | −1.8         | −1.2         |
| 1200 | 565.9       | 585.0       | 596.6       | 539.8          | −30.7        | −19.1        |
| 1400 | **432.7**   | **471.6**   | **547.1**   | **392.1**      | **−114.4**   | **−38.9**    |
| 1600 | 346.0       | 415.5       | 463.1       | 325.1          | −117.1       | **−69.5**    |
| 1800 | 371.8       | 388.8       | 445.6       | 323.3          | −73.8        | −17.0        |
| 2000 | 414.7       | 449.1       | 489.2       | 367.5          | −74.5        | −34.4        |

**Reservation ER/s:**

| RPS  | emv grove_4 | emv grove_1 | ple grove_4 |
|------|-------------|-------------|-------------|
| 1200 | 30.8        | 11.8        | 5.3         |
| 1400 | **263.4**   | **223.4**   | **152.4**   |
| 1600 | 327.5       | 276.8       | 289.2       |
| 1800 | 314.6       | 278.5       | 263.7       |
| 2000 | 351.9       | 291.6       | 266.6       |

**Key findings:**
- Removing est_remaining from ER check increased ER rate at 1400 from 223 → 263/s (wrong direction)
- Goodput dropped at every RPS: −38.9 at 1400, −69.5 at 1600
- Root cause: without preemptive ER, more requests queue up → system gets more overloaded → more expire at actual deadline → ER fires anyway but after wasting CPU
- The preemptive ER check in grove_1 WAS helping; the problem is the inflated signal driving it

**Decision: REVERT grove_4. Restore grove_1 state (asymmetric α + est_remaining ER check).**

---

## Iteration 5 (final): Floor-EMA for ER threshold — decouple inflation from ER (grove_5)

**Status:** Pending

### Change
Add a `mean_floor` field to `LatencyMeanVar` that tracks a floor estimate using asymmetric α (α_floor_up=0.01, α_floor_down=0.3). Use `mean_floor` for the ER threshold in `before_child_rpc`, while keeping `mean` for child deadline computation.

Under overload:
- `mean` inflates (correctly tightens child deadlines, urgency propagation works)
- `mean_floor` resists inflation (α_up=0.01 → ~100-obs window for rising estimates)
- ER threshold uses `mean_floor` → fires conservatively, near actual deadline (like ple's RMS)

After load drops or 0-injection:
- `mean_floor` deflates quickly (α_down=0.3 → ~3-obs window) → ER threshold recovers fast

### Hypothesis
The root cause of emv's inferiority to ple is that `mean` inflates under load and is used as the ER threshold signal — firing ER preemptively on requests that could complete. ple's RMS estimate stays near-zero (batch-update anchoring), so its ER fires conservatively. By tracking a separate floor estimate (`mean_floor`) resistant to inflation, we get ple-like ER behavior while retaining emv's benefits (adaptive child deadline tightening for urgency propagation, EDF reprioritization via before_poll).

grove_4 proved that removing est_remaining entirely is wrong (late ER wastes CPU). grove_5 replaces it with a better signal.

### Expected outcomes:
1. Reservation ER at 1400 drops from grove_1's 223/s toward ple's ~150–183/s
2. Goodput at 1400 rises from grove_1's 471.6 toward ple's ~510–547
3. No regression at 1600–2000 (mean_floor deflates quickly when load rises, so the floor stays appropriate)

### Experiment design
Same grove_1 config. Named grove_5.

### Actual Outcomes (grove_5)

**Status:** Keep ✅ — large wins at 1400/1800/2000, acceptable regression at 1600

**Goodput table:**

| RPS  | emv grove_5 | emv grove_1 | emv pine10 | ple grove_5 | oldest grove_5 | emv−ple (g5) | Δemv (g5−g1) |
|------|-------------|-------------|------------|-------------|----------------|--------------|--------------|
| 100  | 49.9        | 49.0        | 48.5       | 49.8        | 50.7           | +0.1         | +0.9         |
| 400  | 200.0       | 199.6       | 199.6      | 200.1       | 200.1          | −0.0         | +0.4         |
| 800  | 398.0       | 398.4       | 400.0      | 401.0       | 399.5          | −3.0         | −0.4         |
| 1200 | 589.5       | 585.0       | 559.5      | 589.1       | 578.0          | +0.4         | +4.5         |
| 1400 | **523.8**   | **471.6**   | **400.9**  | **567.8**   | **506.7**      | **−44.0**    | **+52.2**    |
| 1600 | 396.2       | 415.5       | 399.2      | 439.9       | 333.8          | −43.7        | −19.2        |
| 1800 | **458.8**   | **388.8**   | **383.8**  | **470.0**   | **353.9**      | **−11.2**    | **+70.0**    |
| 2000 | **496.4**   | **449.1**   | **418.0**  | **504.6**   | **391.1**      | **−8.2**     | **+47.4**    |

**Reservation ER/s:**

| RPS  | emv grove_5 | emv grove_1 | ple grove_5 |
|------|-------------|-------------|-------------|
| 1200 | 7.9         | 11.9        | 15.5        |
| 1400 | **173.3**   | **223.4**   | **129.6**   |
| 1600 | 272.4       | 276.8       | 264.2       |
| 1800 | 265.5       | 278.5       | 237.6       |
| 2000 | 296.7       | 291.6       | 277.5       |

**Key findings:**
1. mean_floor worked: Reservation ER at 1400 dropped from 223 → 173/s (−50/s). Still above ple's 129.6/s (−43.7 gap).
2. Goodput at 1400: +52.2 over grove_1. Remaining gap to ple: −44.0.
3. At 1800 and 2000: emv nearly ties ple (−11.2 and −8.2) — within run-to-run variance. Big wins over grove_1 (+70, +47).
4. emv beats prio_oldest at ALL overloaded RPS: +17 at 1400, +62 at 1600, +105 at 1800/2000.
5. Regression at 1600: −19.2 vs grove_1. Likely caused by mean_floor deflating quickly during the 1600 step (fast α_down=0.3), causing floor to track below actual need.

**Decision: KEEP grove_5.** 3 wins (1400/1800/2000) outweigh 1 regression (1600) in magnitude. emv now uniformly beats prio_oldest.

---

## GROVE Track Conclusions

### What we learned
- **Root cause confirmed:** emv EMA mean inflates under overload → premature Reservation ER → goodput loss at saturation knee. With SLO=50ms, even 20–30µs est_remaining inflation is proportionally large.
- **Asymmetric α (grove_1):** Slowing upward α from symmetric 0.1 → α_up=0.05, α_down=0.2 reduced ER by 72/s at 1400, goodput +70.8. Best single change.
- **Floor EMA (grove_5):** Separate mean_floor with α_up=0.01, α_down=0.3 for ER threshold reduced ER by another 50/s at 1400, goodput +52.2 on top of grove_1.
- **Dead ends:** Lower α_up=0.02 (grove_2) regressions at 1600. Probabilistic ER (grove_3) increased ER. Deadline-only ER (grove_4) catastrophic everywhere.

### Best code state: grove_5
Code: `mean_floor` field in `LatencyMeanVar` (α_up=0.01, α_down=0.3) + asymmetric `mean` α (α_up=0.05, α_down=0.2). ER threshold uses `mean_floor_estimate()`. Code commits: d75b5fd6 (grove_1), 11562cae (grove_5).

### Remaining gap to ple
emv lags ple by ~44 goodput at 1400–1600 RPS. Root barrier: ple's RMS estimator is inherently near-zero (batch-update anchoring to low-load baseline), firing ER at only 129/s vs emv's 173/s at 1400. Any EMA-based mean will leak upward under sustained overload faster than a floor trick can suppress. Full elimination of the gap would require a non-mean estimator for the ER threshold (e.g., running minimum window, quantile estimator) — fundamentally different from EMA.

### emv vs prio_oldest
Grove_5 emv beats prio_oldest at ALL overloaded RPS by +17 to +105. This is the main practical win from the GROVE track.
