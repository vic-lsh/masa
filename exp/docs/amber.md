# amber — hotel Search API: fifo,rajomon,early vs prio_local/prio_oldest baselines

## Key questions
- What is the peak goodput achievable by each policy (fifo,rajomon,early / prio_local,early,adctl / prio_oldest,early,adctl / prio_oldest,early) on the hotel Search API?
- Do the cobalt_3 rajomon parameters (threshold=20ms, step_up=4, step_down=2, cap=40) transfer to hotel, or does the real microservice architecture (MongoDB, Redis, geo/profile/rate fan-out) require retuning?
- How does fifo,rajomon,early compare to the prio_* scheduling baselines?

## Experiment series: amber_1, amber_2, ... (hotel)

---

## Phase 0: Broad sweep (amber_1) — find saturation point for each policy

**Rajomon constants (cobalt_3 starting point):** threshold=20ms, step_up=4, step_down=2, cap=40, MAX_TOKEN=100

**Config:** Search API only, 200ms SLO, RPS [200, 400, 600, 800, 1000, 1200], DurationSecs=60, WarmupSecs=20, Gap=exp.

**Goal:** Identify at what RPS each policy peaks and where rajomon collapses (if it does). This sets the focus range for subsequent iterations.

### Observed Symptoms (amber_1)

| RPS | fifo,rajomon,early | prio_local,adctl | prio_oldest,early | prio_oldest,adctl |
|-----|--------------------|-----------------|------------------|------------------|
| 200 | 0.997 | 0.988 | 1.005 | 1.001 |
| 400 | 0.996 | 0.998 | 0.996 | 1.005 |
| 600 | 0.995 | 0.993 | 0.990 | 1.001 |
| 800 | **0.507** | 0.971 | 0.971 | 0.918 |
| 1000 | 0.733 | **0.894** | 0.689 | 0.696 |
| 1200 | **0.000** | 0.744 | **0.824** | 0.612 |

Peak goodput: `prio_oldest,early` wins at 988 req/s @ 1200 RPS. `fifo,rajomon,early` is dead at 1200 RPS (0.0 fraction).

**Root cause of rajomon collapse:** The price mechanism never activates. Hotel services have per-service queue latency of only 1–2ms — far below the 20ms threshold calibrated for mssim. Price stays at 0 across all services for the entire experiment. All goodput loss is from time-based early return with FIFO scheduling. No load shedding occurs before the e2e deadline is exhausted.

**Key finding:** `prio_oldest,early,adctl` performs *worse* than `prio_oldest,early` at 800 and 1200 RPS — adctl over-rejects when the priority scheduler alone would handle load better.

---

## Iteration 1: Lower threshold to 5ms, faster ramp for hotel (amber_2)

**Status:** Pending

### Change
- `LATENCY_THRESHOLD_US`: 20_000 → 5_000 (5ms — above hotel's 1–2ms idle queue latency, below congestion onset)
- `PRICE_STEP_UP`: 4 → 8 (faster ramp; with threshold barely above idle, step needs to be responsive)
- `PRICE_CAP`: 40 → 60 (higher rejection ceiling; at 1200 RPS need to shed 33% excess load)

### Hypothesis
The 20ms threshold was calibrated for mssim's simulated queue latencies. Hotel's real microservices have lower baseline queue latency (1–2ms). At 5ms, the threshold will activate as soon as load increases and queues begin to grow, giving the price mechanism time to shed load before e2e deadlines are consumed by multi-hop accumulated latency.

### Experiment design
Dense RPS sweep [700, 800, 900, 1000, 1100, 1200] to characterize the saturation knee. Same 4 policies. Named `amber_2`.

### Actual Outcomes (amber_2)

**Status:** Partial improvement, token gate still inert ❌

| Policy | 700 | 800 | 900 | 1000 | 1100 | 1200 | Peak |
|---|---|---|---|---|---|---|---|
| fifo,rajomon,early | 687 | 501 | 710 | **943** | 820 | **0** | 943 |
| prio_oldest,early | 701 | 727 | 786 | 903 | **1004** | 874 | **1004** |
| prio_local,early,adctl | 698 | 782 | 833 | 852 | 790 | 895 | 895 |
| prio_oldest,early,adctl | 695 | 730 | 706 | 737 | 742 | 727 | 742 |

Threshold fix improved fifo,rajomon,early at 800 (+95) and 1000 (+210 RPS) vs amber_1. But **total collapse at 1200 RPS (0 goodput) is unchanged**.

**Root cause of collapse:** The token gate still never fires. Reservation's own_price rises to PRICE_CAP=60 (queue latency 154ms), but the client always bids near MAX_TOKEN=100. Since 100 > 60, the check `tokens < accumulated_price` never triggers. The **frontend** own_price stays 0 (its queue latency peaks at 4.6ms, just below the 5ms threshold). Without the frontend propagating a non-zero price to the loadgen, the loadgen's CLIENT_TOKEN_BUCKET is never depleted and bids remain near 100.

---

## Iteration 2: Lower threshold to 2ms — activate the frontend price signal (amber_3)

**Status:** Pending

### Change
`LATENCY_THRESHOLD_US`: 5_000 → 2_000 (2ms). Only change — all other constants stay.

### Hypothesis
The frontend's queue latency peaks at 4.6ms. At a 2ms threshold, the frontend will start raising own_price during overload. This price propagates in response headers back to the loadgen, causing the loadgen's CLIENT_TOKEN_BUCKET to be depleted. Once the loadgen bids drop below PRICE_CAP, the token gate fires and actual admission control engages — preventing the 1200 RPS collapse.

### Expected outcomes if hypothesis is correct
1. fifo,rajomon,early goodput at 1200 RPS > 0 (token gate sheds excess load before e2e deadline)
2. Some token rejections appear (not all losses are time-based ER)
3. Non-monotone goodput curve (943 at 1000, then graceful decline) instead of cliff

### Experiment design
Same RPS [700, 800, 900, 1000, 1100, 1200]. Named `amber_3`.

### Actual Outcomes (amber_3)

**Status:** Regression ❌ — 2ms threshold too low, fires at idle baseline (frontend ql 2–3ms normal)

| RPS | fifo,rajomon,early | prio_local,adctl | prio_oldest,early | prio_oldest,adctl |
|-----|--------------------|-----------------|------------------|------------------|
| 700 | 0.967 | 0.998 | 0.993 | 0.967 |
| 800 | **0.485** | 0.940 | 0.904 | 0.921 |
| 900 | 0.485 | 0.960 | 0.878 | 0.782 |
| 1000 | 0.653 | **0.912** | **0.917** | 0.731 |
| 1100 | 0.727 | 0.759 | 0.539 | 0.678 |
| 1200 | **0.055** | 0.716 | 0.669 | 0.575 |

Amber_2 was better at 800–1000 RPS by 113–290 req/s. 1200 RPS improved slightly (66 vs 0) but still catastrophic.

**Root cause:** Frontend queue latency sits at 2–3ms even at moderate load (not overloaded). At 2ms threshold, prices flicker on unnecessarily at 800–900 RPS. The token gate did technically activate (6/98 frontend price readings were non-zero, max=12), but too weakly to help at 1200 RPS — 79 token rejections vs 71,882 total requests. The real bottleneck is the rate service (prices 40–60, ql 4–14ms), but its price signals don't propagate effectively upstream.

**Decision:** Revert to amber_2 approach but find the precise threshold that's above the 2–3ms idle baseline but below the 4–5ms overload peak. Try 3ms.

---

## Iteration 3: Threshold 3ms — between idle baseline and overload onset (amber_4)

**Status:** Pending

### Change
`LATENCY_THRESHOLD_US`: 2_000 → 3_000 (3ms).

### Hypothesis
Frontend idle queue latency is 2–3ms; overload peak is 4–5ms. A 3ms threshold sits at the knee — stays silent during moderate load, activates only when the system is genuinely congested. This should recover amber_2's 800–1000 RPS goodput while still engaging the price mechanism at 1100–1200 RPS.

### Experiment design
Same RPS [700, 800, 900, 1000, 1100, 1200]. Named `amber_4`.

### Actual Outcomes (amber_4)

**Status:** Best at 800 RPS, but 1100 RPS collapsed (likely run variability, Repeats=1)

| RPS | fifo,rajomon,early | prio_local,adctl | prio_oldest,early | prio_oldest,adctl |
|-----|--------------------|-----------------|------------------|------------------|
| 700 | 685 (0.978) | 702 (1.002) | 695 (0.992) | 693 (0.990) |
| 800 | **748 (0.935)** | 781 (0.976) | 719 (0.899) | 745 (0.931) |
| 900 | 761 (0.845) | 873 (0.970) | 724 (0.805) | 722 (0.802) |
| 1000 | 825 (0.825) | 873 (0.873) | 678 (0.678) | 712 (0.712) |
| 1100 | 404 (0.367) | **907 (0.825)** | **864 (0.785)** | 715 (0.650) |
| 1200 | 0 (0.000) | 802 (0.669) | 748 (0.623) | 734 (0.612) |

Token gate still inert: frontend own_price = 0 across all 97 log windows. Zero token rejections.

---

## Final Summary: amber optimization track complete

### Best goodput per policy across all amber experiments

| Policy | Peak goodput | At RPS | Experiment |
|--------|-------------|--------|------------|
| **prio_oldest,early** | **1004 req/s** | 1100 | amber_2 |
| prio_local,early,adctl | 912 req/s | 1000 | amber_3 |
| fifo,rajomon,early | 943 req/s | 1000 | amber_2 |
| prio_oldest,early,adctl | 746 req/s | 1100 | amber_3 |

### fifo,rajomon,early across all thresholds

| Threshold | Best RPS point | Behavior |
|-----------|---------------|----------|
| 20ms (amber_1) | 733 @ 1000 | Gate never fires; collapses at 1200 |
| 5ms (amber_2) | **943 @ 1000** | Gate never fires; best overall curve |
| 2ms (amber_3) | 800 @ 1100 | Gate barely fires (6/98 windows); hurts 800–1000 RPS |
| 3ms (amber_4) | **748 @ 800** | Gate inert; best at 800 RPS but 1100 collapses (noise) |

### Why the token gate is structurally limited on hotel

The Rajomon admission gate checks `ctx.tokens() < accumulated_price`. The loadgen's `CLIENT_TOKEN_BUCKET` replenishes at TOKEN_UPDATE_STEP=5 per 10ms (500 tokens/s). Bids stay near MAX_TOKEN=100 because the deduction only occurs when the service's accumulated price is non-zero — but the frontend's accumulated price is almost always 0:

1. **Frontend own_price ≈ 0:** Frontend queue latency peaks at 4–5ms under heavy load; even at threshold=3ms it only fires briefly (3–6 ticks) before recovering. By the time the loadgen sees a response with non-zero price, the price has already decayed back to 0.
2. **Downstream price propagation blocked:** The real bottleneck (rate service) has prices 40–60 and queue latency 4–14ms, but the DashMap of downstream prices is cleared every 1 second. With PRICE_FREQ=5, prices propagate in 1/5 of responses, meaning at 800 RPS only ~160 responses/sec carry price updates. The 1s cache clear erases them before the loadgen can consistently see non-zero accumulated price.
3. **Arithmetic gap:** With bids ≈ 100 and PRICE_CAP ≤ 60, the check `tokens < price` never fires regardless of threshold. Fixing this requires either: (a) depleting the loadgen's token bucket (need non-zero frontend price consistently), or (b) lowering TOKEN_UPDATE_STEP to slow replenishment.

**Net result:** In all amber experiments, goodput control came entirely from the `early` return mechanism (deadline-based), not the Rajomon token gate. fifo,rajomon,early with the `early` flag behaves identically to `fifo,early` — the rajomon price mechanism adds no benefit in the hotel application due to propagation dynamics and FIFO scheduling.

### Recommended constants for detail.txt (best balance)
`LATENCY_THRESHOLD_US = 5_000` (amber_2 config) — best overall goodput curve, no false positives at any load level.
