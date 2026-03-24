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
