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

### Actual Outcomes (basalt_2)

**Status:** Failed ❌ — no improvement over basalt_1

#### Goodput Comparison (basalt_2)

| RPS | adctl | Rajomon-tuned | Delta |
|-----|-------|--------------|-------|
| 100 | 99.5 | 96.2 | +3.4 |
| 400 | 398.0 | 382.2 | +15.8 |
| 800 | 796.0 | 763.5 | +32.5 |
| 1200 | 1194.0 | 1147.4 | +46.6 |
| 1400 | 1386.9 | 1337.3 | +49.6 |
| 1600 | 1530.8 | 1440.7 | +90.1 |
| 1800 | 1619.0 | **172.2** | +1446.8 |
| 2000 | 1680.2 | **189.6** | +1490.7 |

Results are **statistically indistinguishable** from basalt_1 at every load level. The collapse at 1800+ RPS is completely unchanged. At 1600 RPS, performance actually degraded by 65 goodput (1506 → 1441).

### Root cause analysis (post-basalt_2)

The server-side price parameters are **not the binding constraint**. Re-reading the code reveals the true bottleneck:

1. **Per-request token budget is always `rand(1..=100)`** (hardcoded in `libs/masa/src/lib.rs` and `ClientTokenBucket::try_acquire`). The `replenish_amount`/`max_tokens` changes only affect the client-side pool gating (whether to send the request at all), NOT the per-request budget carried through the call graph.

2. **Low-load bleed mechanism:** At price=1 (minimum), a request traversing N services spends N tokens. The hotel call graph has ~5-7 services. Requests drawing 1-4 tokens are guaranteed to be rejected at some downstream service. With uniform(1..=100), ~4% of requests draw ≤4 tokens → explains the constant ~4% loss.

3. **High-load collapse mechanism:** Once any service's price exceeds ~50-100, virtually no request has enough tokens to pass through even the first service. With proportional price increases at 1800 RPS queue latencies (likely 100ms+), prices spike to thousands, far exceeding the max token budget of 100.

**Decision:** Revert the basalt_1 code changes (they had no effect) and proceed to iteration 2 targeting the per-request token budget.

## Iteration 2: Increase per-request token budget (experiment basalt_3)

**Status:** Pending

### Change
Increase the per-request token budget from `1..=100` to `100..=10000` in two places:
1. `libs/masa/src/lib.rs` — initial token assignment for outgoing requests
2. `libs/tonic/tonic/src/masa/context/rajomon.rs` `ClientTokenBucket::try_acquire` — token assignment after client pool admission

Also increase client-side pool to match: `max_tokens = 100000`, `replenish_amount = 10000` (so the pool doesn't become the bottleneck with 10000-token draws).

### Hypothesis
The per-request token budget `1..=100` is the binding constraint for both failure modes:

**Low-load bleed (4% loss):** At minimum price=1, traversing ~5-7 hotel services costs 5-7 tokens. With uniform(1..=100), ~4-7% of requests draw ≤7 tokens and are guaranteed to be rejected at some downstream service regardless of system load. Increasing the floor to 100 means even the lowest-budget request can traverse 100 services at price=1 — eliminating unnecessary rejections.

**High-load collapse:** At 1800 RPS, queue latencies spike to 100ms+, causing prices to spike to hundreds or thousands via proportional increase. With max budget=100, prices only need to exceed 100 to reject ALL traffic. With max budget=10000, prices would need to reach 10000 — giving the price feedback loop much more dynamic range before total collapse. This should allow Rajomon to degrade gracefully (partial rejection) rather than catastrophically (total rejection).

### Expected outcomes if hypothesis is correct:
1. Low-load goodput improves from ~95.5% to >99% of offered load (matching adctl)
2. The catastrophic collapse at 1800–2000 RPS is eliminated or at least delayed to a higher RPS
3. Rajomon achieves >1000 goodput at 2000 RPS

### Experiment design
Same config as basalt_1 (full RPS sweep [100–2000]). Need full sweep to verify both low-load improvement and high-load stability.

### Actual Outcomes (basalt_3)

**Status:** Mixed — low-load fixed, collapse unchanged

#### Goodput Comparison (basalt_3)

| RPS | adctl | Rajomon (100..=10000 tokens) | Delta |
|-----|-------|------------------------------|-------|
| 100 | 99.5 | 99.6 | -0.1 |
| 400 | 398.1 | 398.1 | 0.0 |
| 800 | 795.9 | 796.0 | -0.1 |
| 1200 | 1193.1 | 1193.9 | -0.8 |
| 1400 | 1390.4 | 1388.7 | +1.7 |
| 1600 | 1529.6 | 1486.5 | +43.1 |
| 1800 | 1540.8 | **177.5** | +1363.3 |
| 2000 | 1730.8 | **190.3** | +1540.5 |

#### Comparison: basalt_1 → basalt_3 Rajomon improvement

| RPS | basalt_1 | basalt_3 | Change |
|-----|----------|----------|--------|
| 100-1400 | ~95.5% | ~99.2-99.6% | **+4pp (fixed!)** |
| 1600 | 94.1% | 92.9% | -1.2pp |
| 1800 | 9.6% | 9.9% | unchanged |
| 2000 | 9.5% | 9.5% | unchanged |

**Hypothesis 1 (low-load): CONFIRMED.** Rajomon now matches adctl at low load (99.2-99.6% vs 99.3-99.5%). The token budget floor of 100 eliminated false-positive rejections.

**Hypothesis 2 (collapse): REJECTED.** Collapse is identical. At 1800+ RPS, Search goodput drops to exactly 0.0 while Reservation survives at ~177-190. The larger token budget had no effect because the problem is not token exhaustion — it's that admitted requests are not shed once doomed. adctl early-returns 249+ req/s at 1800 RPS; Rajomon has no equivalent mechanism (rajomon and `early` are mutually exclusive).

**Decision:** Keep the token budget change (it fixed low-load). The collapse is a structural limitation: Rajomon can only control admission, not execution. Without early return, admitted requests that will miss SLO consume CPU until completion, causing cascading queue buildup. Proceed to iteration 3 to try faster price feedback as a partial mitigation.

## Iteration 3: Aggressive price feedback + full propagation (experiment basalt_4)

**Status:** Pending

### Change
On top of the 100..=10000 token budget from iteration 2, tune price feedback for faster reaction:
- `PRICE_PER_EXCESS_MS`: 10 → 50 (5x faster price increase under overload)
- `PRICE_DECREASE_STEP`: 1 → 5 (faster recovery to reduce oscillation)
- `PRICE_PROPAGATION_PROB`: 0.2 → 1.0 (every response propagates price — fastest possible convergence)
- `QUEUE_THRESHOLD_US`: 1000 → 2000 (slight increase to avoid triggering on normal variation)

### Hypothesis
The collapse at 1800 RPS happens because the price feedback loop is too slow to shed load before queues explode. With the current parameters:
- EWMA α=1/4 at 100ms ticks: takes ~400ms to converge on the true queue latency
- 20% propagation: clients only learn about price changes 1 in 5 responses
- PRICE_PER_EXCESS_MS=10: at 100ms queue latency (EWMA=100000µs), increment = ~1000/tick → takes ~10 ticks (1 second) to reach price 10000 (which would reject all traffic with max budget 10000)

During that 1-second convergence window, thousands of requests are admitted and queue up, making the overload worse. By the time prices rise high enough to shed, the system is already in cascade failure.

Making price increase 5x faster and propagation 100% (vs 20%) should reduce the convergence time from ~1s to ~200ms. This may be fast enough to shed load before queue buildup becomes catastrophic. Faster decrease (5 vs 1) reduces oscillation by allowing faster recovery after load drops.

### Expected outcomes if hypothesis is correct:
1. The collapse at 1800 RPS is delayed or softened — Rajomon achieves >500 goodput at 1800 RPS (vs current 177)
2. At 2000 RPS, Rajomon achieves >500 goodput (vs current 190)
3. Low-load performance is unaffected (prices stay at 1 when EWMA < threshold)

### Experiment design
Same config as basalt_1 (full RPS sweep). Focus analysis on 1600-2000 RPS behavior and on whether price oscillation is visible in the early return patterns.

### Actual Outcomes (basalt_4)

**Status:** Failed ❌ — collapse unchanged, marginal improvement at 1600 only

#### Goodput Comparison (basalt_4)

| RPS | adctl | Rajomon (aggressive pricing) | Delta |
|-----|-------|------------------------------|-------|
| 100 | 99.6 | 99.6 | 0.0 |
| 400 | 398.2 | 398.1 | +0.1 |
| 800 | 796.0 | 796.1 | -0.1 |
| 1200 | 1193.5 | 1192.2 | +1.3 |
| 1400 | 1380.2 | 1386.8 | -6.6 |
| 1600 | 1432.3 | 1554.6 | -122.3 |
| 1800 | 1578.9 | **176.2** | +1402.7 |
| 2000 | 1711.5 | **192.8** | +1518.7 |

#### Comparison: basalt_3 → basalt_4 Rajomon

| RPS | basalt_3 | basalt_4 | Change |
|-----|----------|----------|--------|
| 1600 | 1486.5 (92.9%) | 1554.6 (97.2%) | **+68 (+4.3pp)** |
| 1800 | 177.5 (9.9%) | 176.2 (9.8%) | unchanged |
| 2000 | 190.3 (9.5%) | 192.8 (9.6%) | unchanged |

**Hypothesis (faster price feedback prevents collapse): REJECTED.** The 5x price increase, 5x price decrease, 5x propagation probability changes had zero effect on the 1800+ RPS collapse. The only positive effect was a +68 goodput improvement at 1600 RPS (from 92.9% to 97.2%). The collapse at 1800+ is impervious to price tuning.

**At 1800+ RPS:** Search goodput remains exactly 0.0; only Reservation survives at ~176-193. The pattern is identical across basalt_1, basalt_3, and basalt_4.

**Decision:** Revert the price changes (marginal benefit at 1600 doesn't justify diverging from defaults). Keep only the token budget fix from iteration 2 as the best Rajomon configuration. The collapse is a structural limitation, not a tuning problem.

## Iteration 4: Tight client pool + fast EWMA as gateway-level rate limiter (experiment basalt_5)

**Status:** Pending

### Change
On top of the 100..=10000 token budget from iteration 2, change five parameters targeting the feedback loop speed and client-side admission:
1. EWMA α: 1/4 → 1/2 (line 106: `(window_avg + old_ewma) / 2` instead of `(window_avg + 3 * old_ewma) / 4`)
2. Tick interval: 100ms → 50ms (line 187: `Duration::from_millis(50)`)
3. `replenish_amount`: 10000 → 1000 (line 455)
4. `max_tokens`: 100000 → 5000 (line 456)
5. `PRICE_PROPAGATION_PROB`: 0.2 → 1.0 (line 24)

### Hypothesis
Previous iterations changed how fast prices *increase per tick* (basalt_4: 5x PRICE_PER_EXCESS_MS) and *token capacity* (basalt_3: 100x range), but never changed:
- How fast the EWMA *converges to the true queue latency* (α=1/4, 100ms ticks → ~400ms convergence)
- How aggressively the *client-side pool* throttles at the gateway

The client pool with replenish=10000 sustains 1M tokens/sec. Even at price=10000, it admits 100 req/sec/method — it is **never the binding constraint**. All admission happens via per-request token checks deep in the call graph, AFTER requests have entered and consumed CPU.

Making the client pool the primary "fast fuse":
- At price=1 (no overload): replenish=1000 per 10ms → 100K tokens/sec per method. No bottleneck.
- At price=500 (moderate overload): 1000/500 = 2 req/10ms = 200 req/sec per method. Starts shedding excess.
- At price=1000 (heavy overload): 1000/1000 = 1 req/10ms = 100 req/sec per method. Aggressive shedding.

Combined with faster EWMA (α=1/2, 50ms ticks → ~100ms convergence) and 100% price propagation, the system should:
1. Detect overload within 100ms (vs 400ms)
2. Propagate prices to clients immediately (vs 20% chance)
3. Shed excess traffic AT THE GATEWAY (client pool becomes binding) before requests enter the call graph

This is fundamentally different from basalt_2/4 which tuned server-side price magnitude. Here we're making the client pool the active admission controller, rejecting requests before they waste CPU.

### Expected outcomes if hypothesis is correct:
1. The catastrophic collapse at 1800–2000 RPS is softened — Rajomon achieves >500 goodput (vs current ~180)
2. Low-load performance is preserved (prices stay at 1, client pool is not binding)
3. There may be oscillation (admit → prices rise → reject → prices drop → admit) but with shorter cycle time than cascade failure

### Experiment design
Same config as basalt_1 (full RPS sweep [100–2000]). Full sweep needed to verify low-load isn't harmed and to see if the collapse point shifts.

### Actual Outcomes (basalt_5)

**Status:** Failed ❌ — collapse unchanged, slight regression at 1600

#### Goodput Comparison (basalt_5)

| RPS | adctl | Rajomon (tight pool + fast EWMA) | Delta |
|-----|-------|----------------------------------|-------|
| 100 | 99.5 | 99.7 | -0.2 |
| 400 | 398.0 | 397.9 | +0.1 |
| 800 | 795.8 | 796.0 | -0.2 |
| 1200 | 1193.9 | 1191.0 | +2.9 |
| 1400 | 1382.3 | 1385.4 | -3.1 |
| 1600 | 1478.0 | 1453.4 | +24.6 |
| 1800 | 1561.7 | **176.6** | +1385.1 |
| 2000 | 1733.2 | **190.3** | +1542.9 |

#### Comparison: basalt_3 → basalt_5 Rajomon

| RPS | basalt_3 | basalt_5 | Change |
|-----|----------|----------|--------|
| 100–1400 | ~99.5% | ~99.2–99.7% | unchanged |
| 1600 | 1486.5 (92.9%) | 1453.4 (90.8%) | **-33 (-2.1pp)** |
| 1800 | 177.5 (9.9%) | 176.6 (9.8%) | unchanged |
| 2000 | 190.3 (9.5%) | 190.3 (9.5%) | unchanged |

**Hypothesis (gateway-level rate limiting via tight client pool): REJECTED.** The tight client pool, faster EWMA, and 100% price propagation had zero effect on the 1800+ RPS collapse. The only measurable effect was a -33 goodput regression at 1600 RPS (client pool being too aggressive at the transition point). Search goodput remains exactly 0.0 at 1800+.

The client pool is not the binding constraint for the collapse. Even when the pool actively rate-limits at the gateway, the phase transition at 1800 RPS is identical — indicating the collapse is driven by dynamics WITHIN the call graph (admitted requests consuming CPU for work that misses SLO), which no external admission control can address.

**Decision:** Revert all basalt_5 code changes. The tight client pool and fast EWMA provide no benefit and slightly hurt 1600 RPS.

## Iteration 5: Very large token range for gradual shedding (experiment basalt_6)

**Status:** Pending

### Change
Increase per-request token budget from `100..=10000` to `100..=100000` in two places:
1. `libs/masa/src/lib.rs` — initial token assignment
2. `libs/tonic/tonic/src/masa/context/rajomon.rs` `ClientTokenBucket::try_acquire` — token assignment after client pool admission

Also increase client-side pool to match: `max_tokens = 1000000`, `replenish_amount = 100000`.

### Hypothesis
The collapse is caused by a **phase transition** in the token-price interaction. With budget range 100..=10000, the system transitions from "all admitted" to "all rejected" over a very narrow price band:
- At per-hop price=1000, 5-hop Search cost ≈ 10000 → requests with exactly max budget barely survive
- At per-hop price=2000, 5-hop cost ≈ 20000 → exceeds max budget → 100% rejection

This all-or-nothing behavior creates the cliff at 1800 RPS: prices spike past the threshold, ALL Search is rejected, and the system can't find a stable partial-shedding equilibrium.

With 100..=100000 (10x range):
- At per-hop price=2000, 5-hop cost ≈ 10000 → ~90% of requests survive (those with >10000/99900 tokens)
- At per-hop price=5000, 5-hop cost ≈ 25000 → ~75% survive
- At per-hop price=10000, 5-hop cost ≈ 50000 → ~50% survive
- At per-hop price=20000, 5-hop cost ≈ 100000 → ~0% survive

This gives a smooth degradation curve where the price feedback loop can find a stable operating point with partial shedding, rather than oscillating between 100% admission and 100% rejection.

### Expected outcomes if hypothesis is correct:
1. The sharp cliff at 1800 RPS is replaced by gradual degradation — Rajomon achieves >500 goodput at 1800 RPS
2. Low-load performance is preserved (at price=1, min budget of 100 can traverse 100 services)
3. The system may still trail adctl at 2000 RPS but the gap shrinks from 9x to <3x

### Experiment design
Same config as basalt_1 (full RPS sweep [100–2000]).

### Actual Outcomes (basalt_6)

**Status:** Failed ❌ — collapse unchanged, regression at 1600

#### Goodput Comparison (basalt_6)

| RPS | adctl | Rajomon (100..=100000 tokens) | Delta |
|-----|-------|-------------------------------|-------|
| 100 | 99.5 | 99.5 | 0.0 |
| 400 | 398.0 | 398.0 | 0.0 |
| 800 | 796.0 | 796.0 | 0.0 |
| 1200 | 1191.4 | 1191.4 | 0.0 |
| 1400 | 1388.8 | 1388.8 | 0.0 |
| 1600 | 1534.2 | 1393.7 | +140.5 |
| 1800 | 1591.1 | **178.4** | +1412.7 |
| 2000 | 1735.1 | **191.4** | +1543.7 |

#### Comparison: basalt_3 → basalt_6 Rajomon

| RPS | basalt_3 | basalt_6 | Change |
|-----|----------|----------|--------|
| 100–1400 | ~99.5% | ~99.5% | unchanged |
| 1600 | 1486.5 (92.9%) | 1393.7 (87.1%) | **-93 (-5.8pp)** |
| 1800 | 177.5 (9.9%) | 178.4 (9.9%) | unchanged |
| 2000 | 190.3 (9.5%) | 191.4 (9.6%) | unchanged |

**Hypothesis (wider token range prevents phase transition): REJECTED.** The 10x larger token budget had zero effect on the 1800+ collapse and caused a -93 goodput regression at 1600 RPS. The wider budget doesn't help because the problem isn't the granularity of per-request shedding — it's that once queue latencies spike, ALL services raise prices simultaneously, and the accumulated cost across the call graph exceeds ANY budget.

**Decision:** Revert to 100..=10000 token budget (basalt_3 configuration). The 100..=100000 range is strictly worse.

## Iteration 6: Fast oscillation via ultra-responsive EWMA + rapid price recovery (experiment basalt_7)

**Status:** Pending

### Change
On top of the 100..=10000 token budget from iteration 2:
1. EWMA α: 1/4 → 7/8 (`(7 * window_avg + old_ewma) / 8` instead of `(window_avg + 3 * old_ewma) / 4`)
2. Tick interval: 100ms → 50ms
3. `PRICE_DECREASE_STEP`: 1 → 5000 (very fast price recovery when queues clear)
4. `PRICE_PROPAGATION_PROB`: 0.2 → 1.0 (instant price propagation)

### Hypothesis
The current EWMA (α=1/4, 100ms tick) has a critical flaw for overload recovery: even when queues EMPTY (window_avg=0), the EWMA takes ~6 ticks (600ms) to drop below QUEUE_THRESHOLD_US. During those 600ms, prices KEEP INCREASING (because EWMA > threshold), even though the queue is already empty. This creates a one-way ratchet: prices spike instantly under overload but take hundreds of seconds to recover (price decreases at 1/tick, and recovery only starts after EWMA drops below threshold/2).

With α=7/8: when queue empties, new_ewma = old_ewma/8. From 100000µs: tick 1 → 12500, tick 2 → 1562, tick 3 → 195 (below threshold/2=500). Recovery triggers in **3 ticks (150ms)** instead of 6+ ticks.

With PRICE_DECREASE_STEP=5000: once recovery triggers, price drops from 10000 → 5000 → 1 in 2 ticks. Total recovery time: **5 ticks (250ms)** from queue emptying.

This creates fast oscillation: overload → prices spike → all rejected → queue drains (100ms) → EWMA drops (150ms) → prices crash (100ms) → requests admitted → overload again. Cycle time ≈ 500ms. During each cycle, ~200ms of admit window where system operates within capacity (~1500 RPS goodput). Average goodput ≈ 0.4 × 1500 = 600. Even this conservative estimate would be 3.4x better than current 178.

### Expected outcomes if hypothesis is correct:
1. Rajomon achieves >500 goodput at 1800 RPS (vs current 178)
2. Visible oscillation pattern in the data (goodput variance increases)
3. Low-load performance preserved (EWMA stays near 0, prices stay at 1)

### Experiment design
Same config as basalt_1 (full RPS sweep [100–2000]).

### Actual Outcomes (basalt_7)

**Status:** Failed ❌ — collapse unchanged

#### Goodput Comparison (basalt_7)

| RPS | adctl | Rajomon (fast oscillation) | Delta |
|-----|-------|----------------------------|-------|
| 100 | 99.5 | 99.5 | 0.0 |
| 400 | 398.0 | 398.0 | 0.0 |
| 800 | 796.1 | 796.1 | 0.0 |
| 1200 | 1193.1 | 1193.1 | 0.0 |
| 1400 | 1390.8 | 1390.8 | 0.0 |
| 1600 | 1534.6 | 1499.7 | +34.9 |
| 1800 | 1589.0 | **176.1** | +1412.9 |
| 2000 | 1730.8 | **190.6** | +1540.2 |

#### Comparison: basalt_3 → basalt_7 Rajomon

| RPS | basalt_3 | basalt_7 | Change |
|-----|----------|----------|--------|
| 100–1400 | ~99.5% | ~99.5% | unchanged |
| 1600 | 1486.5 (92.9%) | 1499.7 (93.7%) | +13 (+0.8pp) |
| 1800 | 177.5 (9.9%) | 176.1 (9.8%) | unchanged |
| 2000 | 190.3 (9.5%) | 190.6 (9.5%) | unchanged |

**Hypothesis (fast oscillation yields higher average goodput): REJECTED.** Ultra-responsive EWMA (α=7/8), 50ms ticks, PRICE_DECREASE_STEP=5000, and 100% propagation had zero effect on the 1800+ collapse. The +13 at 1600 is marginal. The oscillation approach fails because the overload-to-collapse transition is not a slow feedback loop problem — it's a sharp phase transition that no oscillation frequency can smooth.

**Decision:** Revert. The marginal +13 at 1600 doesn't justify 4 parameter changes from defaults.

## Iteration 7: Aggressive pricing + fast EWMA (experiment basalt_8)

**Status:** Pending

### Change
Combine the best 1600 RPS result (basalt_4's aggressive pricing) with fast EWMA:
1. `PRICE_PER_EXCESS_MS`: 10 → 50 (from basalt_4)
2. `PRICE_DECREASE_STEP`: 1 → 5 (from basalt_4)
3. `QUEUE_THRESHOLD_US`: 1000 → 2000 (from basalt_4)
4. `PRICE_PROPAGATION_PROB`: 0.2 → 1.0 (from basalt_4)
5. EWMA α: 1/4 → 1/2 (moderate responsiveness, not as noisy as 7/8)
6. Tick: 100ms → 50ms

### Hypothesis
The 1800+ collapse is structural and unfixable by parameter tuning (confirmed by 6 iterations). The goal shifts to **maximizing pre-collapse performance**. basalt_4's aggressive pricing achieved 1554.6 (97.2%) at 1600 — the best Rajomon result at that load point. Adding faster EWMA may further improve the 1400-1600 range by enabling quicker price adaptation to load changes. Faster EWMA may also push the collapse point slightly higher (e.g., from 1750 to 1850 effective RPS).

### Expected outcomes:
1. 1600 RPS: Rajomon achieves >1550 goodput (matching or exceeding basalt_4's best)
2. 1400 RPS: Rajomon maintains ~99% goodput
3. 1800+ RPS: collapse persists (~180 goodput)

### Experiment design
Same config as basalt_1 (full RPS sweep). Focus on 1400-1800 range.

### Actual Outcomes (basalt_8)

**Status:** Regression ❌ — worst 1600 result, collapse unchanged

#### Rajomon Comparison: basalt_3 vs basalt_4 vs basalt_8

| RPS | basalt_3 | basalt_4 (best 1600) | basalt_8 (combined) |
|-----|----------|---------------------|---------------------|
| 1200 | 1193.9 | 1192.2 | 1193.6 |
| 1400 | 1388.7 | 1386.8 | 1390.6 |
| 1600 | 1486.5 | **1554.6** | **1291.4** |
| 1800 | 177.5 | 176.2 | 176.6 |
| 2000 | 190.3 | 192.8 | 191.6 |

**The combination is worse than its parts.** basalt_4's aggressive pricing alone achieved 1554.6 at 1600. Adding faster EWMA caused over-reaction: the faster signal tracking amplifies aggressive price feedback, creating oscillations that reject too many requests at the saturation point. Rajomon at 1600 dropped to 1291 — the worst result across all iterations.

**Decision:** Revert. basalt_4's configuration (aggressive pricing only, default EWMA) remains the best for 1600 RPS, but it was previously reverted because the 1600 improvement didn't justify diverging from defaults.

## Iteration 8: Minimal rejection — very low price sensitivity (experiment basalt_9)

**Status:** Pending

### Change
1. `PRICE_PER_EXCESS_MS`: 10 → 1 (10x slower price escalation)
2. `QUEUE_THRESHOLD_US`: 1000 → 50000 (prices only increase above 50ms queue latency)
3. `PRICE_DECREASE_STEP`: 1 → 50 (fast recovery when queues drop below 25ms)

### Hypothesis
All previous iterations explored the "more rejection" direction (faster/stronger price response). This iteration explores the opposite extreme: minimal rejection. With PRICE_PER_EXCESS_MS=1 and threshold=50000µs, prices barely increase even under significant queueing. The system relies on natural backpressure (slow responses → fewer new requests) rather than active rejection.

This tests whether Rajomon's rejection mechanism is actually HURTING at the collapse point by causing instability (reject → drain → admit → overload → reject) that's worse than simply processing everything and accepting SLO violations. If goodput at 1800 improves beyond ~180, it suggests the rejection mechanism itself contributes to the collapse.

### Expected outcomes if hypothesis is correct:
1. 1800 RPS: goodput changes from ~178 to either higher (rejection was hurting) or ~0 (confirming rejection is the only thing keeping Reservation alive)
2. 1600 RPS: may regress slightly (less rejection → more queue buildup)
3. Low-load: unchanged (prices ~1 regardless)

### Experiment design
Same config as basalt_1 (full RPS sweep). This is a diagnostic experiment to understand whether less rejection helps or hurts.

### Actual Outcomes (basalt_9)

**Status:** Diagnostic — confirmed collapse is independent of rejection

#### Goodput Comparison (basalt_9)

| RPS | adctl | Rajomon (minimal rejection) | Delta |
|-----|-------|------------------------------|-------|
| 100 | 99.5 | 99.5 | 0.0 |
| 400 | 398.0 | 398.0 | 0.0 |
| 800 | 796.0 | 795.9 | +0.1 |
| 1200 | 1193.5 | 1193.5 | 0.0 |
| 1400 | 1389.9 | 1389.9 | 0.0 |
| 1600 | 1508.9 | 1508.9 | 0.0 |
| 1800 | 1575.3 | **175.2** | +1400.1 |
| 2000 | 1708.4 | **191.2** | +1517.2 |

#### Comparison: basalt_3 → basalt_9 Rajomon

| RPS | basalt_3 | basalt_9 | Change |
|-----|----------|----------|--------|
| 100–1400 | ~99.5% | ~99.5% | unchanged |
| 1600 | 1486.5 (92.9%) | 1508.9 (94.3%) | +22 (+1.4pp) |
| 1800 | 177.5 (9.9%) | 175.2 (9.7%) | unchanged |
| 2000 | 190.3 (9.5%) | 191.2 (9.6%) | unchanged |

**Key finding:** With near-zero price sensitivity (PRICE_PER_EXCESS_MS=1, QUEUE_THRESHOLD_US=50000), Rajomon's price-based rejection is essentially disabled — yet the collapse at 1800+ RPS is **identical** (~175 goodput, 0 Search). This proves definitively that the collapse is NOT caused by Rajomon's rejection mechanism. It is a property of the underlying system under overload without early return.

The +22 goodput at 1600 confirms the default pricing is slightly over-zealous at the saturation boundary, but the effect is minor.

**Decision:** Revert. This was a diagnostic experiment confirming the structural nature of the collapse.

## ⚠ Bug invalidation notice (2026-03-13)

**Commit ef86f1a0 ("fix: add queue latency timing to FIFO scheduler queue") revealed that all experiments basalt_1 through basalt_9 ran with a broken Rajomon price mechanism.** The FIFO queue was missing `set_enqueue_time()` and `record_queue_lat()` calls, so `obtain_task_queue_latency()` always returned 0. This meant the EWMA never registered any queue latency, prices never increased from their initial value, and Rajomon's admission control was effectively disabled in all experiments.

**What this invalidates:**
- All conclusions about price tuning (iterations 1, 3, 4, 6, 7, 8) — the price changes had no effect because the input signal was always zero
- The "structural collapse" diagnosis — the collapse was simply the system running without any load shedding
- The "Rajomon can only control admission, not execution" conclusion — untested because admission control was never active
- The basalt_9 diagnostic ("collapse is independent of rejection") — trivially true because rejection was already broken

**What remains valid:**
- basalt_3 (token budget 100→10000 fixing low-load bleed) — this was about per-request token math at price=0, which was the actual operating point

**Current codebase state:** Token budget `100..=10000` (from basalt_3) with all other Rajomon parameters at defaults, plus the queue latency bug fix. Experiments resume from iteration 9 below.

---

## Iteration 9: Re-baseline with working price feedback (experiment basalt_10)

**Status:** Pending

### Change
No code change — the bug fix (ef86f1a0) is already committed. This is a clean re-run of basalt_1's config with working queue latency timing.

### Hypothesis
With functional queue latency reporting, Rajomon's EWMA-based price feedback will actually activate under load. At overload (1800+ RPS), queue latencies will drive prices up, causing the token-budget admission checks to reject excess traffic. The previous "collapse to ~180 goodput" was Rajomon running with zero admission control — with working prices, the system should shed load and achieve meaningfully higher goodput.

The magnitude of improvement is uncertain since we've never seen Rajomon with working prices. Two possibilities:
1. **Rajomon works well**: prices rise proportionally to overload, partial shedding occurs, goodput at 1800+ is substantially higher than 180 (maybe 500-1000+)
2. **Rajomon still collapses**: even with working prices, the feedback loop is too slow or the price-token interaction creates phase transitions similar to what we hypothesized (but couldn't test) before

### Expected outcomes:
1. Low-load (100-1400 RPS): goodput should remain ~99.5% (prices stay near 0, no false rejections)
2. High-load (1800-2000 RPS): goodput should increase substantially from ~180 (old broken baseline)
3. We'll see actual price dynamics for the first time, informing future tuning directions

### Experiment design
Full RPS sweep [100, 400, 800, 1200, 1400, 1600, 1800, 2000] — same as basalt_1. We need the full sweep as a clean baseline with working Rajomon.

### Actual Outcomes (basalt_10)

**Status:** Complete ✅ — dramatically different from broken runs

#### Goodput Comparison (basalt_10)

| RPS | adctl | Rajomon (fixed) | Rajomon (broken, basalt_3) | Delta (adctl - fixed) |
|-----|-------|-----------------|---------------------------|----------------------|
| 100 | 99.6 | 99.5 | 99.6 | +0.1 |
| 400 | 397.8 | 363.9 | 398.1 | +33.9 |
| 800 | 795.6 | 609.1 | 796.0 | +186.5 |
| 1200 | 1127.8 | 677.1 | 1193.9 | +450.7 |
| 1400 | 1154.8 | 663.0 | 1388.7 | +491.8 |
| 1600 | 1100.3 | 719.2 | 1486.5 | +381.1 |
| 1800 | 1537.4 | 752.5 | 177.5 | +784.9 |
| 2000 | 1466.8 | 784.7 | 190.3 | +682.1 |

#### Goodput by Request Type at High Load

| RPS | Type | adctl | Rajomon (fixed) |
|-----|------|-------|-----------------|
| 1800 | Search | 785.3 | 482.2 |
| 1800 | Reservation | 752.1 | 270.3 |
| 2000 | Search | 713.5 | 495.7 |
| 2000 | Reservation | 753.3 | 289.1 |

### Key Findings

1. **The catastrophic collapse is gone.** Rajomon at 1800 now gives 752.5 goodput (vs 177.5 broken) — a 4.2x improvement. Search goodput is no longer zero.

2. **New problem: over-rejection at ALL load levels.** Rajomon starts shedding at 400 RPS (363.9 vs 397.8 adctl). At 800 RPS — well below saturation — it achieves only 76% goodput (609/796). The price feedback is too sensitive with default QUEUE_THRESHOLD_US=1000µs (1ms).

3. **Cascading rejections across service layers.** Rajomon early-returns at both the frontend AND downstream reservation service. A request can be rejected at the downstream service even when the frontend would have let it through. adctl only early-returns at the frontend layer, avoiding this compounding effect.

4. **The previous "structural limitation" diagnosis was wrong.** The collapse was simply Rajomon running with no load shedding at all (broken queue latency). With working prices, Rajomon can shed load — it just needs to be tuned to shed less aggressively.

**Decision:** Keep the bug fix. Proceed to iteration 10 to reduce price sensitivity, starting with a higher QUEUE_THRESHOLD_US to prevent price activation at sub-saturation load levels.

## Iteration 10: Raise queue threshold to reduce false-positive rejections (experiment basalt_11)

**Status:** Pending

### Change
1. `QUEUE_THRESHOLD_US`: 1000 → 10000 (10ms — only activate price increases when queue latency exceeds 10ms, which indicates genuine congestion rather than normal queuing)
2. `PRICE_DECREASE_STEP`: 1 → 10 (faster recovery when queues clear, preventing hysteresis from transient spikes)

### Hypothesis
The default QUEUE_THRESHOLD_US=1000µs (1ms) triggers price increases at queue latencies that are normal for a multi-service system under moderate load. Even at 400 RPS (well below saturation), the hotel call graph's ~5-7 services create enough queuing to exceed 1ms, driving prices up and causing false-positive rejections.

Raising the threshold to 10ms means prices only increase when queue latency clearly indicates congestion. At 400-800 RPS, queue latencies should be well under 10ms, so prices remain at 1 and no false rejections occur. At 1600+ RPS, genuine congestion will push latencies above 10ms, activating load shedding.

The faster price decrease (10 vs 1) prevents a scenario where a brief load spike pushes prices up and they take too long to come down, rejecting requests after the spike has passed.

### Expected outcomes if hypothesis is correct:
1. Low-to-moderate load (100-1200 RPS): Rajomon matches adctl (~99.5% goodput) — no false rejections
2. Near saturation (1400-1600 RPS): Rajomon achieves >90% goodput (vs current 47-45%)
3. Overload (1800-2000 RPS): Rajomon maintains or improves on the 752-785 goodput from basalt_10

### Experiment design
Full RPS sweep. Need to verify both that low-load false rejections are eliminated and that high-load shedding still works.

### Actual Outcomes (basalt_11)

**Status:** Mixed — low-load fixed, high-load regressed

#### Goodput Comparison (basalt_11)

| RPS | adctl | Rajomon (b11, 10ms) | Rajomon (b10, 1ms) | Best Rajomon |
|-----|-------|--------------------|--------------------|-------------|
| 100 | 99.5 | 99.6 | 99.5 | b11 |
| 400 | 397.8 | **397.8** | 363.9 | **b11** |
| 800 | 795.4 | **795.6** | 609.1 | **b11** |
| 1200 | 1186.3 | **1148.8** | 677.1 | **b11** |
| 1400 | 1267.0 | 829.3 | 663.0 | **b11** |
| 1600 | 1167.8 | 718.5 | 719.2 | tie |
| 1800 | 1543.9 | **368.5** | **752.5** | **b10** |
| 2000 | 1668.9 | 736.0 | 784.7 | **b10** |

**Hypothesis partially confirmed.** Raising the threshold to 10ms completely eliminated false rejections at 400-800 RPS (now matching adctl perfectly) and dramatically improved 1200 RPS (1149 vs 677). However, high-load performance regressed badly — 1800 RPS dropped from 752 to 368.

**Root cause:** The 10ms threshold delays price activation too long. By the time queue latencies cross 10ms, the system is deeply overloaded. The delayed reaction creates a more chaotic feedback loop: massive queue buildup → sudden price spike → reject everything → queues drain → prices crash → admit everything → repeat. The faster PRICE_DECREASE_STEP=10 amplifies this oscillation.

**The crossover point is around 1600 RPS** — below that, 10ms is better; above, 1ms is better. The optimal threshold is somewhere between 1ms and 10ms.

**Decision:** Revert. Try 5ms threshold with default PRICE_DECREASE_STEP=1 (slower recovery = less oscillation).

## Iteration 11: Moderate threshold (5ms) with slow price recovery (experiment basalt_12)

**Status:** Pending

### Change
1. `QUEUE_THRESHOLD_US`: 10000 → 5000 (5ms — midpoint between 1ms and 10ms)
2. `PRICE_DECREASE_STEP`: 10 → 1 (back to default — slow recovery prevents oscillation)

### Hypothesis
basalt_10 (1ms threshold) and basalt_11 (10ms threshold) bracket the optimal operating point:
- 1ms: over-rejects at moderate load (400-1200), but handles overload (1800) with 752 goodput
- 10ms: perfect at moderate load, but overload detection is too late (1800: 368 goodput)

5ms should split the difference: normal queuing at 400-800 RPS stays under 5ms (no false rejections), while genuine congestion at 1400+ RPS crosses 5ms earlier than 10ms (faster shedding activation).

Reverting PRICE_DECREASE_STEP to 1 (from 10) addresses the oscillation problem in basalt_11: when prices drop too fast, the system cycles between full admission and full rejection. Slow recovery (step=1) keeps prices elevated after a spike, maintaining steady-state partial shedding rather than oscillating.

### Expected outcomes if hypothesis is correct:
1. 400-800 RPS: ~99% goodput (matching adctl, no false rejections)
2. 1200-1400 RPS: >90% goodput (better than basalt_10's 56-47%)
3. 1800-2000 RPS: >700 goodput (matching or exceeding basalt_10's 752-785)

### Experiment design
Full RPS sweep.

### Actual Outcomes (basalt_12)

**Status:** Mixed — low-load fixed, high-load inconsistent

#### Goodput Comparison across all threshold experiments

| RPS | adctl | Rajomon 1ms (b10) | Rajomon 10ms (b11) | Rajomon 5ms (b12) | Best Rajomon |
|-----|-------|-------------------|--------------------|--------------------|-------------|
| 100 | 99.6 | 99.5 | 99.6 | 99.6 | tie |
| 400 | 397.8 | 363.9 | 397.8 | 397.7 | 5ms/10ms |
| 800 | 795.6 | 609.1 | 795.6 | 795.4 | 5ms/10ms |
| 1200 | 1133.5 | 677.1 | **1148.8** | 800.7 | **10ms** |
| 1400 | 1353.6 | 663.0 | 829.3 | **883.5** | **5ms** |
| 1600 | 1113.8 | **719.2** | 718.5 | 613.4 | **1ms** |
| 1800 | 1416.0 | **752.5** | 368.5 | 750.6 | **1ms** |
| 2000 | 1405.3 | 784.7 | 736.0 | **876.9** | **5ms** |

**Hypothesis partially confirmed.** 5ms threshold eliminates low-load false rejections (matching 10ms at 400-800 RPS) but doesn't consistently improve high-load behavior. No single threshold dominates — the best config changes at every RPS level.

**Root cause:** The problem isn't the threshold alone — it's that `PRICE_PER_EXCESS_MS=10` causes prices to escalate too quickly once the threshold is crossed. At 1200 RPS with 5ms threshold, even moderate queue latency (8ms) causes prices to rise by 30/tick, reaching 300 after 1 second. With a 5-hop call graph, the accumulated cost is 1500 tokens — rejecting ~15% of requests unnecessarily. The cascading rejection across service layers compounds this: a request can be rejected at the frontend AND at downstream services independently.

**Decision:** Keep 5ms threshold (best overall compromise), but reduce price sensitivity to address over-shedding.

## Iteration 12: Low price sensitivity with 5ms threshold (experiment basalt_13)

**Status:** Pending

### Change
1. `PRICE_PER_EXCESS_MS`: 10 → 2 (5x slower price escalation — prices need sustained high queue latency to reach rejection-level values)
2. Keep `QUEUE_THRESHOLD_US` at 5000 (5ms)
3. Keep `PRICE_DECREASE_STEP` at 1

### Hypothesis
The current `PRICE_PER_EXCESS_MS=10` causes prices to reach rejection-level values too quickly once queue latency exceeds the threshold. In a 5-hop call graph with token budget 100-10000:
- At price=200 per hop: total cost = 1000 → rejects ~9% of requests (those with budget <1000)
- At price=1000 per hop: total cost = 5000 → rejects ~49% of requests
- At price=2000 per hop: total cost = 10000 → rejects ~100%

With `PRICE_PER_EXCESS_MS=10` and 5ms threshold, at queue latency=15ms: excess=10ms, price increase=100/tick. After 10 ticks (1s), price=1000/hop → 50% rejection. This is too aggressive for moderate overload.

With `PRICE_PER_EXCESS_MS=2`: same scenario yields price increase=20/tick, reaching 200 after 1s → only ~9% rejection. Prices would need to accumulate for ~5 seconds at sustained high queue latency to reach 50% rejection. This gives the system much more time to find a stable partial-shedding equilibrium rather than oscillating between over-admission and over-rejection.

At severe overload (1800+ RPS) with queue latencies of 50-100ms+: excess=45-95ms, price increase=90-190/tick → still reaches rejection-level prices (1000+) within ~5-10 ticks (500ms-1s). Load shedding still activates, just with a longer ramp.

### Expected outcomes if hypothesis is correct:
1. 400-800 RPS: ~99.5% goodput (unchanged — prices stay near 0)
2. 1200 RPS: >1000 goodput (reduced over-shedding, closer to adctl's 1133)
3. 1400-1600 RPS: >800 goodput (gentler shedding curve)
4. 1800-2000 RPS: ≥750 goodput (shedding still activates, possibly better due to more stable equilibrium)

### Experiment design
Full RPS sweep.

### Actual Outcomes (basalt_13)

**Status:** Best Rajomon result yet ✅

#### Goodput Comparison (basalt_13)

| RPS | adctl | Rajomon b13 (5ms, price=2) | Best prior Rajomon | Improvement |
|-----|-------|---------------------------|-------------------|-------------|
| 100 | 99.5 | 99.5 | 99.6 | — |
| 400 | 397.9 | 397.8 | 397.8 | — |
| 800 | 795.4 | 795.5 | 795.6 | — |
| 1200 | 1138.5 | **1037.9** | 800.7 (b12) | **+237** |
| 1400 | 1089.2 | **987.1** | 883.5 (b12) | **+104** |
| 1600 | 1167.6 | 712.1 | 750.6 (b12) | -38 |
| 1800 | 1281.1 | **892.3** | 752.5 (b10) | **+140** |
| 2000 | 1269.2 | 874.2 | 876.9 (b12) | -3 |

**Hypothesis confirmed.** Reducing PRICE_PER_EXCESS_MS from 10 to 2 substantially improved the over-shedding at 1200 RPS (+237 goodput vs b12) and achieved the best-ever Rajomon result at 1800 RPS (892.3). The gentler price escalation allows the system to find partial-shedding equilibria rather than oscillating.

**Rajomon vs adctl gap:** At 1200-1400 RPS, Rajomon now achieves ~91% of adctl's goodput. At 1800-2000 RPS, it achieves ~69-70% of adctl. The gap at 1600 RPS (61%) remains the weakest point.

**Remaining weakness at 1600 RPS:** Rajomon drops to 712.1 at 1600 (61% of adctl's 1167.6). This is the transition zone where the system crosses from "partially loaded" to "overloaded." The early return breakdown shows aggressive shedding at both frontend and reservation services, suggesting the cascading rejection problem (rejections at multiple service layers) is still the main drag.

**Decision:** Keep. This is the best overall Rajomon configuration. Proceed to iteration 13 for one final attempt to close the 1600 RPS gap.

## Iteration 13: Full price propagation for faster client-side adaptation (experiment basalt_14)

**Status:** Pending

### Change
1. `PRICE_PROPAGATION_PROB`: 0.2 → 1.0 (every response carries price information)
2. Keep `QUEUE_THRESHOLD_US` at 5000, `PRICE_PER_EXCESS_MS` at 2, `PRICE_DECREASE_STEP` at 1

### Hypothesis
With `PRICE_PROPAGATION_PROB=0.2`, clients only learn about price changes from 1 in 5 responses. This creates a lag: when prices rise due to queue buildup, 80% of responses still carry stale (low) price info. Clients continue sending requests at the old rate, causing the server to reject them via per-hop token checks deep in the call graph — wasting the CPU used to process them through upstream services.

With 100% propagation, clients learn about price increases immediately. The client-side token pool (replenish=10000, max=100000) becomes a more effective gateway: at high prices, the pool depletes faster and clients self-throttle BEFORE sending requests into the call graph. This shifts rejection from expensive deep-in-call-graph token failures to cheap client-side pool exhaustion.

At 1600 RPS (the weakest point), this should help because:
- Faster price signal → clients self-limit sooner → fewer requests enter the call graph → less CPU wasted on doomed requests → more CPU available for requests that will succeed
- The improvement should be most visible at the transition zone (1400-1800) where the system oscillates between capacity and overload

### Expected outcomes if hypothesis is correct:
1. 1600 RPS: >800 goodput (up from 712)
2. 1200-1400 RPS: maintained or improved (faster price updates help fine-grained shedding)
3. 1800-2000 RPS: maintained or improved
4. Low-load: unchanged (prices stay near 0)

### Experiment design
Full RPS sweep.

### Actual Outcomes (basalt_14)

**Status:** Regression ❌ — full propagation causes over-reaction

#### Goodput Comparison (basalt_14)

| RPS | adctl | Rajomon b14 (prop=1.0) | Rajomon b13 (prop=0.2) | Delta (b14-b13) |
|-----|-------|------------------------|------------------------|-----------------|
| 100 | 99.5 | 99.5 | 99.5 | 0.0 |
| 400 | 397.7 | 397.9 | 397.8 | +0.1 |
| 800 | 795.5 | 795.0 | 795.5 | -0.5 |
| 1200 | 1146.6 | **1089.4** | 1037.9 | +51.5 |
| 1400 | 1043.9 | **703.6** | 987.1 | **-283.5** |
| 1600 | 1047.8 | **783.6** | 712.1 | +71.5 |
| 1800 | 1399.6 | 871.3 | 892.3 | -21.0 |
| 2000 | 1294.9 | **690.4** | 874.2 | **-183.8** |

**Hypothesis rejected.** Full price propagation causes a "price storm" — every response carries price info, amplifying the feedback loop. At 1400 RPS, Rajomon drops from 987 to 704 (-29%). At 2000, from 874 to 690 (-21%). The modest improvements at 1200 (+51) and 1600 (+72) are far outweighed by the regressions.

**Root cause:** The 0.2 propagation probability in the default config acts as a natural damper on price oscillation. With 100% propagation, price increases and decreases propagate instantly, creating rapid oscillation: prices spike → mass rejection → queues drain → prices crash → mass admission → prices spike again. The 20% sampling smooths this cycle by introducing lag, which actually helps stability.

**Decision:** Revert. basalt_13 (propagation=0.2) remains the best configuration.

---

## Post-bugfix Assessment (iterations 9–13)

### Summary of post-bugfix iterations

| Iteration | Config | 100-800 | 1200 | 1400 | 1600 | 1800 | 2000 |
|-----------|--------|---------|------|------|------|------|------|
| 9 (b10) | 1ms threshold, defaults | 91-99.5% | 677 (56%) | 663 (47%) | 719 (45%) | 752 (42%) | 785 (39%) |
| 10 (b11) | 10ms threshold, decrease=10 | **99.5%** | **1149 (96%)** | 829 (59%) | 718 (45%) | 369 (21%) | 736 (37%) |
| 11 (b12) | 5ms threshold | **99.5%** | 801 (67%) | 884 (63%) | 613 (38%) | 751 (42%) | 877 (44%) |
| **12 (b13)** | **5ms, price=2** | **99.5%** | **1038 (87%)** | **987 (71%)** | **712 (45%)** | **892 (50%)** | **874 (44%)** |
| 13 (b14) | + propagation=1.0 | *(blocked — disk full)* |

### Best Rajomon configuration (post-bugfix)

`QUEUE_THRESHOLD_US=5000, PRICE_PER_EXCESS_MS=2, PRICE_DECREASE_STEP=1` (iteration 12/basalt_13). Token budget `100..=10000` from the pre-bugfix iteration 2.

| RPS | adctl (best) | Rajomon (tuned) | Gap | Rajomon % of adctl |
|-----|-------------|-----------------|-----|--------------------|
| 100–800 | ~99.5% | ~99.5% | **none** | 100% |
| 1200 | ~1138 | ~1038 | ~100 | **91%** |
| 1400 | ~1089 | ~987 | ~102 | **91%** |
| 1600 | ~1168 | ~712 | ~456 | **61%** |
| 1800 | ~1281 | ~892 | ~389 | **70%** |
| 2000 | ~1269 | ~874 | ~395 | **69%** |

### Updated answers to the key questions

1. **How does Rajomon compare to adctl?** With the queue latency bug fixed and tuned parameters, Rajomon matches adctl at low-to-moderate load (≤800 RPS) and achieves 69-91% of adctl's goodput at overload. The gap is largest at 1600 RPS (61%) — the transition zone. This is a dramatic improvement from the pre-bugfix state where Rajomon collapsed to ~10% goodput at 1800+.

2. **Can hyperparameter tuning close the gap?** Substantially but not fully. Tuning improved Rajomon from 42% of adctl (default params, 1ms threshold) to 70% of adctl at 1800 RPS. The remaining gap is structural: adctl sheds only at the frontend (one rejection point), while Rajomon rejects at every service independently, causing cascading rejections that waste CPU on partially-processed requests. Additionally, adctl uses `early` return to abort in-flight doomed requests — a mechanism Rajomon cannot access.

3. **What is the mechanistic difference?** The same two mechanisms identified pre-bugfix remain the structural gap, but their relative importance has changed:
   - **Cascading rejection:** Rajomon's per-service price-based rejection causes requests to be rejected at both the frontend AND downstream services. This wastes the CPU spent processing the request through upstream services before rejection. adctl concentrates rejection at the frontend, avoiding this waste. This is now the **primary** gap — visible in early return breakdowns where Rajomon rejects at 4+ methods across 2 services while adctl rejects at 2 methods on the frontend only.
   - **Early return:** adctl+early aborts in-flight requests past their deadline, reclaiming CPU. Rajomon cannot use `early` (mutually exclusive). This contributes to the gap but is secondary to the cascading rejection problem.

## Iteration 14: Fix price accumulation to match paper (experiment basalt_15)

**Status:** Pending
**Code commit:** dbf7fa72

### Change
Three bugs fixed in how Rajomon propagates and uses prices:

1. **Price accumulation:** `inject_price_to_response` now sends `accumulated_price(method) = local_price + max_downstream_child_price` instead of just the local price. Upstream services and clients now see the true end-to-end cost of the call path.

2. **Additive instead of max:** New `accumulated_price()` uses `local + downstream` (additive) instead of the old `max(local, downstream)`. Added `child_price()` for outbound checks (returns cached accumulated price from child response).

3. **No double-charging:** `check_inbound` now charges only the LOCAL price (not accumulated). `check_outbound` charges the child's cached accumulated price. Total deduction per hop = local + child_accumulated, matching the paper's model where each service charges its local cost and the upstream verifies total budget.

### Hypothesis
The cascading rejection problem (Rajomon rejects at both frontend AND downstream services) exists because upstream services don't know the true downstream cost. With price accumulation:

- When the search service is overloaded, its high price propagates to the frontend's `accumulated_price`
- The frontend now charges `local_frontend + downstream_search_price` via outbound check BEFORE sending to search
- Requests that can't afford the full path cost are rejected at the frontend, not after wasting CPU traversing upstream services
- This pushes rejection to the earliest possible point (the paper's core design goal), matching adctl's behavior of rejecting only at the frontend

### Expected outcomes if hypothesis is correct:
1. Rejection shifts from downstream services to the frontend (matching adctl's pattern)
2. Less wasted CPU on partially-processed requests → higher goodput at 1200-1800 RPS
3. Low-load unchanged (prices near 1, accumulated price still low)

### Experiment design
Full RPS sweep. Focus on early return breakdown to verify rejection shifts to frontend.

### Actual Outcomes (basalt_15 + basalt_16)

Two sub-experiments tested the fixes independently and together:

**basalt_15 — price accumulation only (still deducting tokens):**

| RPS | adctl | Rajomon b15 | Rajomon b13 (ref) | Delta b15-b13 |
|-----|-------|-------------|-------------------|---------------|
| 100-800 | ~99.5% | ~99.5% | ~99.5% | 0 |
| 1200 | 1152.5 | 1089.7 | 1037.9 | +51.8 |
| 1400 | 1069.1 | 826.7 | 987.1 | **-160.4** |
| 1600 | 1093.4 | 820.1 | 712.1 | **+108.0** |
| 1800 | 1100.6 | 867.4 | 892.3 | -6.8 |
| 2000 | 1247.0 | 869.5 | 874.2 | -22.8 |

Rejection still cascades to downstream services. Price accumulation alone doesn't fix the cascading problem because token deduction at each hop still drains the budget.

**basalt_16 — price accumulation + compare-not-deduct tokens:**

| RPS | adctl | Rajomon b16 | Rajomon b13 (ref) | Delta b16-b13 |
|-----|-------|-------------|-------------------|---------------|
| 100-800 | ~99.5% | 99.7 / 397.9 / **767.9** | ~99.5% | -27 at 800 |
| 1200 | 1150.2 | **164.6** | 1037.9 | **-873** |
| 1400 | 1001.0 | **186.8** | 987.1 | **-800** |
| 1600 | 1060.7 | **216.9** | 712.1 | **-495** |
| 1800 | 1307.9 | **244.3** | 892.3 | **-648** |
| 2000 | 1123.5 | **280.0** | 874.2 | **-594** |

Catastrophic collapse at 1200+ RPS. **However, the rejection pattern shifted correctly:** 99%+ of rejections are at the frontend (matching adctl), with near-zero downstream rejection. The problem is the system rejects TOO MUCH at the frontend.

**Root cause of b16 collapse:** The token budget range (100-10000) was tuned for per-hop local prices (~1-100). With price accumulation, the frontend's accumulated price = local + max(downstream) → easily reaches 1000-5000 at moderate overload. With uniform token budget 100-10000, a large fraction of requests have tokens < accumulated_price and are rejected. The token budget range needs to be scaled up proportionally to the accumulated price magnitude. Additionally, the client-side pool (replenish=10000 per 10ms) deducts accumulated prices which are now much larger, causing it to rate-limit too aggressively.

**Decision:** Revert both changes. The price accumulation fix correctly shifts rejection to the frontend (matching the paper's design), but requires re-tuning the token budget range and client pool to account for the larger accumulated prices. This is a valid direction but needs parameter co-optimization in a follow-up.

**Status:** Reverted

## Iteration 15: Price accumulation + scaled token range (experiment basalt_17)

**Status:** Pending
**Code commit:** 9a7eb94b

### Change
Re-apply the price accumulation fix (dbf7fa72) and compare-not-deduct model (2cf524cb) from iteration 14, plus scale token parameters:
1. Token range: `100..=10000` → `100..=100000` (10x wider ceiling)
2. Client `max_tokens`: `100000` → `10000000` (100x)
3. Client `replenish_amount`: `10000` → `1000000` (100x)

### Hypothesis
basalt_16 proved the price accumulation model correctly pushes 99%+ of rejection to the frontend. The collapse was caused by accumulated prices (1000-5000 at moderate overload) exceeding the token budget ceiling (10000), causing >50% rejection.

With `T_max=100000`, the shedding curve becomes:
- Accumulated price 100 → 0% rejection (no overload, prices ~1 per hop)
- Accumulated price 50000 → ~50% rejection (moderate overload)
- Accumulated price 100000 → ~100% rejection (severe overload)

With PRICE_PER_EXCESS_MS=2 and depth ~3-5, accumulated prices at moderate overload should be ~3000-5000, giving only ~3-5% rejection. At heavy overload (queue 100ms+), accumulated ~10000-30000, giving ~10-30% rejection. This should match the gradual shedding curve needed.

The 100x client pool scaling (replenish=1M per 10ms = 100M tokens/sec) ensures the client pool is never the bottleneck even at high accumulated prices.

### Expected outcomes:
1. Low load (100-800 RPS): ~99.5% goodput (prices ~1, no rejection)
2. Moderate overload (1200-1600): >1000 goodput (gentle shedding, much better than b16's collapse)
3. Heavy overload (1800-2000): >800 goodput (controlled shedding)
4. Rejection pattern: 99%+ at frontend (matching adctl)

### Experiment design
Full RPS sweep.

### Actual Outcomes (basalt_17)

**Status:** Mixed — best-ever 1200 RPS, but collapse persists at 1600+

#### Goodput Comparison (basalt_17)

| RPS | adctl | Rajomon b17 (accum+scaled) | Rajomon b13 (no accum) | Rajomon b16 (accum+small) |
|-----|-------|---------------------------|------------------------|--------------------------|
| 100 | 100.0 | 99.6 | 99.5 | 99.7 |
| 400 | 397.8 | 398.0 | 397.8 | 397.9 |
| 800 | 786.7 | **795.4** | 795.5 | 767.9 |
| 1200 | 759.9 | **1173.0** | 1037.9 | 164.6 |
| 1400 | 1015.1 | 888.6 | 987.1 | 186.8 |
| 1600 | 1442.8 | **197.8** | 712.1 | 216.9 |
| 1800 | 1210.5 | **185.4** | 892.3 | 244.3 |
| 2000 | 1534.9 | **406.3** | 874.2 | 280.0 |

**Scaling the token range moved the collapse from 1200 to 1400 RPS**, but didn't eliminate it. At 1200 RPS, b17 achieves the best Rajomon result ever (1173, actually beating adctl's 760!). But at 1600+ it collapses to ~185-406 goodput — worse than b13's 712-892.

**Rejection pattern is 99%+ frontend** (correct), with only tiny downstream leakage (0.1-3.0 req/s at reservation vs hundreds at frontend). The accumulation model successfully pushes rejection to the gateway.

**Root cause of the persistent collapse:** The price accumulation model creates a **sharp phase transition**. Below the critical load (1400 RPS), accumulated prices stay within the token range → gentle shedding → good goodput. Above it, queue latencies spike → accumulated prices grow rapidly (additive across depth) → quickly exceed most of the token range → near-total rejection → catastrophic collapse. The non-accumulated model (b13) doesn't have this amplification, so it degrades more gracefully.

**Decision:** Revert. The price accumulation model is fundamentally more brittle at high load because price amplification across the call graph depth creates cliff-edge dynamics. b13 (no accumulation, QUEUE_THRESHOLD_US=5000, PRICE_PER_EXCESS_MS=2) remains the most robust configuration.

## Iteration 16: Gentler pricing + higher token floor (experiment basalt_18)

**Status:** Pending

### Change
1. `PRICE_PER_EXCESS_MS`: 2 → 1 (half the price growth rate)
2. Token range: `100..=10000` → `1000..=10000` (raise floor from 100 to 1000)
   - In `libs/masa/src/lib.rs` and `libs/tonic/tonic/src/masa/context/rajomon.rs`

### Hypothesis
b13's weakness is over-shedding at 1200-1400 RPS (1038 and 987 vs adctl's 1138 and 1089). Two mechanisms contribute:
1. **Price escalation too fast:** PRICE_PER_EXCESS_MS=2 means prices reach rejection-level values quickly. Halving to 1 doubles the time needed, giving more room for partial shedding equilibrium.
2. **Low-budget requests rejected unnecessarily:** With uniform(100, 10000), ~9% of requests have budget <1000. At per-hop price=500 (moderate overload), these requests are guaranteed rejected even though the system has capacity. Raising the floor to 1000 ensures every request can traverse at least 1 service at price=1000 before rejection.

At heavy overload (1800+ RPS), prices reach 2000-5000+ per hop. With ceiling=10000, requests with budget <5000 are rejected → ~40% shedding. This should still provide effective load shedding.

### Expected outcomes:
1. 1200-1400 RPS: >1050 goodput (closer to adctl)
2. 1800-2000 RPS: ≥850 goodput (maintained or slightly improved)
3. Low load: unchanged

### Experiment design
Full RPS sweep.

---

## Assessment (pre-bug-fix, now invalidated)

### Summary of all iterations

| Iteration | Change | Low-load effect | High-load (1800+) effect | 1600 RPS |
|-----------|--------|----------------|--------------------------|----------|
| Baseline (basalt_1) | Default Rajomon | 95.5% (−4pp vs adctl) | Collapse (9.5%) | 1506 (94.1%) |
| 1 (basalt_2) | Server-side price tuning | No change | No change | 1441 (-65) |
| 2 (basalt_3) | Token budget 100..=10000 | **99.5% (matches adctl)** | No change | 1487 (92.9%) |
| 3 (basalt_4) | Aggressive price feedback | No change | No change | **1555 (97.2%)** |
| 4 (basalt_5) | Tight client pool + fast EWMA | No change | No change | 1453 (-33) |
| 5 (basalt_6) | Very large token range (100..=100000) | No change | No change | 1394 (-93) |
| 6 (basalt_7) | Ultra-responsive EWMA + rapid recovery | No change | No change | 1500 (+13) |
| 7 (basalt_8) | Aggressive pricing + fast EWMA combined | No change | No change | **1291 (worst)** |
| 8 (basalt_9) | Minimal rejection (diagnostic) | No change | No change | 1509 (+22) |

### Parameter space explored

| Parameter | Default | Values tested | Best value |
|-----------|---------|---------------|------------|
| QUEUE_THRESHOLD_US | 1000 | 1000, 2000, 5000, 50000 | 1000 (default) |
| PRICE_PER_EXCESS_MS | 10 | 1, 10, 50 | 10 (default) or 50 (for 1600 only) |
| PRICE_DECREASE_STEP | 1 | 1, 5, 10, 50, 5000 | 1 (default) |
| PRICE_PROPAGATION_PROB | 0.2 | 0.2, 0.5, 1.0 | 0.2 (default) |
| EWMA α | 1/4 | 1/4, 1/2, 7/8 | 1/4 (default) |
| Tick interval | 100ms | 50ms, 100ms | 100ms (default) |
| Token range | 1..=100 | 1..=100, 100..=10000, 100..=100000 | **100..=10000** |
| Client replenish | 100 | 100, 1000, 10000, 100000 | 10000 |
| Client max_tokens | 1000 | 1000, 5000, 100000, 1000000 | 100000 |

---

## Iteration 16: basalt_16 — gentler pricing + token floor 1000

### Hypothesis

Reducing `PRICE_PER_EXCESS_MS` from 2 to 1 makes pricing less aggressive, so transient queue spikes produce smaller price bumps and fewer false rejections. Raising the token floor from 100 to 1000 (`1000..=10000`) gives each request a larger minimum budget, making it harder for downstream price accumulation to exhaust tokens before the request completes its call graph. Together, these changes should reduce unnecessary shedding at moderate load (1200-1600 RPS) while still allowing rejection under genuine overload.

### Changes
- `PRICE_PER_EXCESS_MS`: 2 → 1
- Token range (client + server): `100..=10000` → `1000..=10000`

### Expected outcome
1. 1200-1600 RPS: fewer false rejections, goodput closer to adctl
2. 1800-2000 RPS: may not improve (structural gap), but should not regress
3. Low load: unchanged

### Experiment design
Full RPS sweep.

---

### Best Rajomon configuration

Token budget `100..=10000` (iteration 2) with all other parameters at defaults. This is the simplest effective configuration.

| RPS | adctl (best) | Rajomon (tuned) | Gap |
|-----|-------------|-----------------|-----|
| 100–1400 | ~99.5% | ~99.5% | **none** |
| 1600 | ~95% | ~93% | ~2pp |
| 1800 | ~88% | **~10%** | **~78pp** |
| 2000 | ~86% | **~10%** | **~76pp** |

### Answer to the key questions

1. **How does Rajomon compare to adctl?** At low-to-moderate load (≤1600 RPS), Rajomon with tuned token budgets matches adctl's goodput. At overload (≥1800 RPS), Rajomon suffers catastrophic collapse (~10% goodput) while adctl degrades gracefully (~86% goodput). The gap at 2000 RPS is **~1500 goodput (9x)**.

2. **Can hyperparameter tuning close the gap?** Only at low load. Token budget tuning eliminated the ~4% low-load bleed. The overload collapse is **completely impervious to parameter tuning** — 8 iterations testing every dimension of the parameter space (price sensitivity, EWMA responsiveness, token budgets, client pool sizing, propagation probability, tick intervals) all produce the same ~178 goodput at 1800 RPS and ~190 at 2000 RPS. The iteration 8 diagnostic (minimal rejection) proved that the collapse occurs even when Rajomon's rejection mechanism is essentially disabled — confirming it is a property of the system, not of Rajomon's tuning.

3. **What is the mechanistic difference?** adctl has two mechanisms Rajomon lacks:
   - **Early return:** adctl+early sheds 250+ requests/sec at high load by aborting in-flight requests that have exceeded their SLO deadline. This reclaims CPU for requests that can still succeed. Rajomon cannot use `early` (mutually exclusive feature flag).
   - **End-to-end cost awareness:** adctl uses latency estimates (`est_mean_var`) to make informed admission decisions based on predicted end-to-end cost. Rajomon's admission is based on per-method queue latency EWMA — a local signal that cannot predict end-to-end behavior in a multi-service call graph.

   The collapse mechanism is: at 1800 RPS, queue latencies spike → admitted Search requests (which fan out to ~5 services) consume CPU processing work that will miss SLO → this crowds out Reservation requests → queue latencies spike further → positive feedback loop. adctl breaks this loop by aborting doomed work; Rajomon lets it run to completion. **No parameter tuning can compensate for this structural gap.**
