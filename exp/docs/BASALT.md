# BASALT — adctl vs Rajomon admission control comparison

## Key questions
- How does Rajomon's admission control compare to adctl when both use FIFO scheduling on the hotel workload?
- Can Rajomon's hyperparameters be tuned to close the gap with adctl (if one exists)?
- What are the mechanistic differences in how adctl and Rajomon shed load under overload?

## Context

FORGE established that `fifo,early,adctl,est_mean_var` achieves 1755 goodput at 2000 RPS on the coral_ext_2 workload (Search SLO=200ms, Reservation SLO=50ms). Rajomon is an alternative admission control mechanism from the NSDI'25 paper that uses per-method token-bucket rate limiting driven by queue latency EWMA, with probabilistic price piggybacking on responses. Unlike adctl, Rajomon is mutually exclusive with `early` return — it implements its own admission/rejection mechanism.

**Policies under test:**
- `fifo,early,adctl,est_mean_var` — adctl admission control with early return and latency estimation
- `fifo,rajomon` — Rajomon admission control with token-bucket rate limiting

**Rajomon default hyperparameters:**
- `QUEUE_THRESHOLD_US = 1000` (1ms queue latency threshold)
- `PRICE_PER_EXCESS_MS = 10` (cost per excess ms above threshold)
- `PRICE_DECREASE_STEP = 1` (price decrease when queue is low)
- `PRICE_PROPAGATION_PROB = 0.2` (20% chance of piggybacking price on response)
- Client: `replenish_amount = 100`, `max_tokens = 1000`, 10ms replenishment interval
- Initial tokens per request: random 1..=100

## Experiment series: basalt_1, basalt_2, ... (hotel)

### Baseline: basalt_1 (adctl vs rajomon, coral_ext_2 config)

**Config:** Search SLO=200ms, Reservation SLO=50ms, RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000]. Two policies: `fifo,early,adctl,est_mean_var` and `fifo,rajomon`.

**Goal:** Establish baseline performance comparison between adctl and Rajomon admission control with default hyperparameters.

### Results: basalt_1

**Status:** Complete ✅

#### Goodput Comparison

| RPS | adctl (fifo,early,adctl) | Rajomon (fifo,rajomon) | Delta | Winner |
|-----|--------------------------|------------------------|-------|--------|
| 100 | 99.7 | 95.5 | +4.2 | adctl |
| 400 | 398.1 | 382.8 | +15.3 | adctl |
| 800 | 796.0 | 765.9 | +30.1 | adctl |
| 1200 | 1193.7 | 1148.2 | +45.5 | adctl |
| 1400 | 1390.3 | 1338.8 | +51.5 | adctl |
| 1600 | 1525.4 | 1505.7 | +19.7 | adctl |
| 1800 | 1581.9 | **172.5** | +1409.4 | adctl |
| 2000 | 1716.1 | **189.0** | +1527.1 | adctl |

#### Goodput as % of Offered Load

| RPS | adctl | Rajomon | Gap |
|-----|-------|---------|-----|
| 100 | 99.7% | 95.5% | +4.2pp |
| 400 | 99.5% | 95.7% | +3.8pp |
| 800 | 99.5% | 95.7% | +3.8pp |
| 1200 | 99.5% | 95.7% | +3.8pp |
| 1400 | 99.3% | 95.6% | +3.7pp |
| 1600 | 95.3% | 94.1% | +1.2pp |
| 1800 | 87.9% | **9.6%** | +78.3pp |
| 2000 | 85.8% | **9.5%** | +76.3pp |

#### Goodput by Request Type at High Load

| RPS | Type | adctl | Rajomon |
|-----|------|-------|---------|
| 1800 | Reservation | 767.7 | 172.5 |
| 1800 | Search | 814.2 | **0.0** |
| 2000 | Reservation | 892.5 | 189.0 |
| 2000 | Search | 823.7 | **0.0** |

### Key Findings

1. **Rajomon catastrophically collapses at 1800 RPS.** Goodput drops from 1506 (at 1600 RPS) to 173 (at 1800 RPS) — an 89% drop. All Search requests fail. This is a cliff, not graceful degradation.

2. **adctl degrades gracefully.** From 1600 to 2000 RPS, goodput goes 1525 → 1582 → 1716. It actually *increases* goodput under overload by aggressively shedding requests that would miss SLO.

3. **Rajomon bleeds goodput even at low load.** At 100–1400 RPS (below saturation), Rajomon achieves only ~95.5–95.7% vs adctl's ~99.3–99.7%. Rajomon needlessly rejects ~4% of requests at every service, even when lightly loaded.

4. **The collapse appears to be a phase transition** caused by positive feedback: once prices spike (proportional increase of +10 per excess ms), they decrease by only 1 per tick — the asymmetric price movement traps the system in a high-price state. With only 20% propagation probability, price *decreases* propagate even more slowly.

5. **Root cause of low-load bleed:** Initial token budget is random(1..=100), and the token cost per request is also random(1..=100). With per-service admission checks, a request traversing multiple services has a high compound probability of being rejected at at least one.

## Iteration 1: Symmetric price steps + higher threshold + larger token budget (experiment basalt_2)

**Status:** Pending

### Change
Tune Rajomon hyperparameters to address both the catastrophic collapse and low-load bleed:
- `QUEUE_THRESHOLD_US`: 1000 → 5000 (less sensitive to normal queuing variation)
- `PRICE_DECREASE_STEP`: 1 → 10 (symmetric with PRICE_PER_EXCESS_MS, preventing hysteresis trap)
- `PRICE_PROPAGATION_PROB`: 0.2 → 0.5 (faster price signal propagation, especially for recovery)
- Client `replenish_amount`: 100 → 500 (more tokens available, reducing low-load rejections)
- Client `max_tokens`: 1000 → 5000 (higher ceiling to match replenishment)

### Hypothesis
The collapse at 1800 RPS is caused by the 10:1 asymmetry between price increase (+10 per excess ms) and decrease (-1 per tick). When queue latency spikes transiently, prices ratchet up quickly but recover extremely slowly, trapping the system in a high-price state that rejects almost everything. Making the decrease step symmetric (10) should allow prices to recover after transient spikes. Raising the threshold from 1ms to 5ms means normal queuing won't trigger price increases at all. Higher propagation probability (50%) ensures price *decreases* reach clients faster.

The low-load bleed is caused by small token budgets relative to request costs. Increasing replenishment 5x should reduce unnecessary rejections when the system is lightly loaded.

### Expected outcomes if hypothesis is correct:
1. The catastrophic collapse at 1800–2000 RPS is eliminated; Rajomon achieves >1000 goodput at 2000 RPS
2. Low-load goodput improves from ~95.5% to >98% of offered load
3. Rajomon still trails adctl at high load (adctl has fundamentally better end-to-end visibility) but the gap shrinks to <200 goodput at 2000 RPS

### Experiment design
Same config as basalt_1 (full RPS sweep [100–2000], Search SLO=200ms, Reservation SLO=50ms). The full sweep is needed to verify both that the collapse is fixed (1800–2000) and that low-load performance improves (100–1400).
