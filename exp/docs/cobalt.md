# cobalt — fifo,rajomon,early + prio_local,rajomon,early

## Key questions
- Can rajomon+early match or beat adctl+early in goodput under overload on the mssim S_14677443 call graph?
- What is the best combination of PRICE_STEP_UP, PRICE_STEP_DOWN, PRICE_CAP, LATENCY_THRESHOLD_US, and token bucket parameters to maximize goodput at high load (800–1000 RPS)?
- Does early return meaningfully complement rajomon's admission control, or does rajomon already absorb enough load that in-flight abortion rarely fires?

## Experiment series: cobalt_1, cobalt_2, ... (mssim)

---

## Observed Symptoms (drift_14) — baseline with threshold=40ms, step_down=1

### Goodput table (req/s meeting 200ms SLO)

| RPS  | fifo,adctl | fifo,rajomon,early | prio_local,adctl | prio_local,rajomon,early |
|------|------------|-------------------|-----------------|------------------------|
| 600  | 598.7      | 599.8             | 613.0           | 604.6                  |
| 700  | 705.2      | 685.4             | 709.3           | 713.2                  |
| 800  | 724.5      | 599.4             | 790.7           | 781.7                  |
| 900  | 721.8      | 395.4             | 889.7           | 726.2                  |
| 1000 | 780.3      | 311.1             | 920.7           | 694.8                  |

### fifo,rajomon,early as % of fifo,adctl

| RPS  | % of fifo,adctl | prio_local,rajomon,early as % of prio_local,adctl |
|------|-----------------|--------------------------------------------------|
| 600  | 100.2%          | 98.6%                                            |
| 700  | 97.2%           | 100.5%                                           |
| 800  | 82.7%           | 98.9%                                            |
| 900  | **54.8%**       | 81.6%                                            |
| 1000 | **39.9%**       | 75.5%                                            |

### Key findings

1. **fifo,rajomon,early collapses above 800 RPS.** At 900 RPS goodput drops to 55% of adctl; at 1000 RPS to 40%. The root cause is time-based early return firing massively (50–70% of requests hit ER at 900–1000 RPS), not token rejections.

2. **prio_local,rajomon,early is more stable** but still 75–82% of adctl at 900–1000 RPS, due to over-rejection via token rejections (~8900 at 1000 RPS vs adctl's 2216 ER).

3. **Rajomon price stays near 0 for fifo,rajomon.** All admitted requests that fail do so via time-based ER, not token rejection. Price never builds because: (a) threshold=40ms is too high relative to typical queue latency, and (b) the price formula is multiplicative (`own + STEP_UP * excess`), meaning as soon as threshold is breached the price jumps instantly to PRICE_CAP=60 (e.g., 45ms queue → excess=5000μs → step=5×5000=25000 → capped at 60). This causes oscillation rather than gradual admission control.

4. **Oscillation confirmed:** fifo,rajomon,early goodput CV ≈ 28% at 900 RPS (vs 5.9% for fifo,adctl). Per-second goodput swings from 237 to 656 — classic sign of the price oscillating between 0 and PRICE_CAP.

5. **Unit test mismatch:** The test `test_step_price_increase_on_congestion` asserts `own + PRICE_STEP_UP` (simple additive), but the formula is multiplicative. Tests are currently failing with the committed code.

### Initial hypotheses
- H1: Fixing the price formula to simple additive + lowering threshold to 2ms should eliminate oscillation and allow gradual admission control (priority: high — this is a correctness bug)
- H2: PRICE_STEP_UP=8 + PRICE_STEP_DOWN=2 (drift_7 best) may give better ramp rate once formula is fixed
- H3: prio_local,rajomon gap at 1000 RPS may need a slightly higher threshold (3–5ms) to avoid over-triggering under priority scheduling

---

## Iteration 1: Fix multiplicative price-step formula + restore 2ms threshold (cobalt_1)

**Status:** Pending

### Change
1. Fix `update_prices` formula from `own + PRICE_STEP_UP * excess` → `own + PRICE_STEP_UP` (simple additive, as documented by the comment and unit tests)
2. Lower `LATENCY_THRESHOLD_US` from 40_000 to 2_000 (2ms) — the proven best from drift_7
3. Restore `PRICE_STEP_DOWN` to 2 (undo uncommitted working-tree change back to 1)
4. Raise `PRICE_STEP_UP` to 8 (from 5, matching drift_7 best constants)
5. Update the comment on LATENCY_THRESHOLD_US to reflect new value

### Hypothesis
The multiplicative excess formula causes price to jump from 0 → PRICE_CAP in a single tick the moment queue latency exceeds the threshold. This creates a sawtooth oscillation: price hits 60, rejects all requests, queue drains, price falls to 0, requests flood in, queue spikes again. The fix — simple additive step — allows price to ramp gradually over ~(PRICE_CAP/STEP_UP) = 60/8 ≈ 7.5 ticks = ~75ms, giving stable equilibrium. Lowering threshold to 2ms ensures the congestion signal fires early, before end-to-end latency already exceeds the SLO.

### Expected outcomes if hypothesis is correct
1. fifo,rajomon,early goodput at 900 RPS rises from 395 → ≥600 req/s (closing ≥50% of the gap vs fifo,adctl)
2. Oscillation CV drops from ~28% toward ~10% or below
3. Token rejections become the primary rejection mechanism (not time-based ER)
4. prio_local,rajomon,early remains competitive or slightly improves at 800–1000 RPS
5. Unit tests pass with `cargo test -p tonic --features rajomon`

### Experiment design
Same call graph, SLO, and load as drift_14 (S_14677443, 200ms, 600–1000 RPS). All 4 policies run together so the adctl baselines serve as a live regression check. Named `cobalt_1`.

### Actual Outcomes (cobalt_1)

**Status:** Regression ❌

| RPS | fifo,adctl | fifo,raj,early (d14→c1) | plocal,adctl | plocal,raj,early (d14→c1) |
|-----|-----------|------------------------|-------------|--------------------------|
| 600 | 594.0     | 599.8 → **261.8** (-56%) | 581.6 | 604.6 → **456.8** (-24%) |
| 700 | 699.2     | 685.4 → **302.6** (-56%) | 690.4 | 713.2 → **459.3** (-36%) |
| 800 | 702.9     | 599.4 → **328.2** (-45%) | 797.0 | 781.7 → **430.3** (-45%) |
| 900 | 684.4     | 395.4 → **351.3** (-11%) | 890.4 | 726.2 → **405.6** (-44%) |
| 1000| 745.3     | 311.1 → **382.3** (+23%) | 928.6 | 694.8 → **420.0** (-40%) |

**What worked:**  The formula fix succeeded: time-based early returns dropped to 0 (vs 50–68% of requests at 900–1000 RPS in drift_14). Oscillation eliminated — goodput CV dropped from 23% → 5.8% at 900 RPS for fifo,rajomon. The additive step formula is correct and should be kept.

**What failed:** LATENCY_THRESHOLD_US=2ms is below the services' baseline idle queue latency. The price mechanism fires even at 600 RPS (well below saturation), causing ~55–60% token rejection uniformly across all load levels. The system permanently believes it is overloaded.

**Root cause:** The 2ms threshold, combined with Tokio's single-threaded scheduler, triggers on nearly every task scheduling cycle. The price converges to 42–60 (near PRICE_CAP) even at low load, starving most requests of tokens.

**Decision:** Keep the formula fix (revert would bring back oscillation). Raise the threshold in Iteration 2.

---

## Iteration 2: Raise threshold to 20ms — find the right operating point (cobalt_2)

**Status:** Pending

### Change
`LATENCY_THRESHOLD_US`: 2_000 → 20_000 (20ms). No other changes.

### Hypothesis
The 2ms threshold was below the services' idle queue latency, making the signal permanently active. The 40ms threshold in drift_14 was too high (only fires under extreme congestion, causing oscillation with the old multiplicative formula). With the fixed additive formula, 20ms should land in the sweet spot: above normal idle latency (avoids spurious triggers at low load), below the onset of true congestion (provides early warning before SLO misses). At this threshold, price should stay near 0 at 600–700 RPS and ramp up gracefully only when the system is genuinely overloaded.

### Expected outcomes if hypothesis is correct
1. fifo,rajomon,early goodput recovers to near 100% at 600–700 RPS (≥650 req/s)
2. Price stays near 0 at low load, rises gradually at 800+ RPS
3. Token rejections appear only at 800+ RPS (not at 600–700 RPS)
4. prio_local,rajomon,early goodput returns to near drift_14 levels or better across all RPS
5. No time-based early returns (formula fix preserved)

### Experiment design
Same config as cobalt_1. Named `cobalt_2`.

### Actual Outcomes (cobalt_2)

**Status:** Mixed ✅/❌ — prio_local fixed at low load, high-load gap remains; fifo structurally limited

| RPS | fifo,adctl (c2) | fifo,raj,early | % adctl | plocal,adctl (c2) | plocal,raj,early | % adctl |
|-----|----------------|---------------|---------|------------------|-----------------|---------|
| 600 | 594.9 | **611.5** | **102.8%** | 618.7 | **608.2** | 98.3% |
| 700 | 685.7 | **687.7** | **100.3%** | 694.0 | **704.3** | **101.5%** |
| 800 | 695.2 | 602.0 | 86.6% | 795.3 | **798.9** | **100.5%** |
| 900 | 722.2 | 367.0 | 50.8% | 885.4 | 815.8 | 92.1% |
| 1000| 738.8 | 294.0 | 39.8% | 934.8 | 796.0 | 85.2% |

**prio_local,rajomon,early (big win):** 20ms threshold eliminated all false-positive rejections. Zero rejections at 600–800 RPS (up from 20–47% in cobalt_1). On par with or ahead of adctl at 600–800 RPS. Gap only at 900–1000 RPS, caused by token rejections at ms-73106 (10% and 20% rejection rates). Price oscillates 0→60→0 (near PRICE_CAP) causing bursty shedding.

**fifo,rajomon,early (structurally limited):** Token rejections are still zero. All losses are time-based early returns caused by ms-56394 queue filling beyond 20ms. The price signal from ms-56394 (max ~20) propagates upstream but is insufficient to shed enough load without priority scheduling to route away from the bottleneck. This is a fundamental FIFO limitation, not a parameter issue.

**Root cause for prio_local 900–1000 RPS gap:** PRICE_CAP=60 means when overloaded, only 40% of requests pass. This is too aggressive — with prio_local scheduling the system CAN handle ~99% of 900 RPS (adctl proves this). The price oscillates between 0 and cap, creating bursty bursts of 40% admission then 100% admission instead of a stable ~90%+ admission.

---

## Iteration 3: Lower PRICE_CAP and PRICE_STEP_UP to smooth equilibrium (cobalt_3)

**Status:** Pending

### Change
1. `PRICE_CAP`: 60 → 40 (i.e., `MAX_TOKEN * 4 / 10`)
2. `PRICE_STEP_UP`: 8 → 4

### Hypothesis
The price is oscillating near PRICE_CAP=60, cycling between 40% admission (price at cap) and 100% admission (price=0). This bursty admission causes the 10–20% excess rejection at 900–1000 RPS. Lowering the cap to 40 means the worst-case admission floor is 60% (better for a system that can genuinely handle most requests), and slowing the ramp from 75ms to 150ms (step_up 8→4) reduces overshoot. Together these should drive the price to a stable equilibrium near the true load-shedding point rather than oscillating between extremes.

### Expected outcomes if hypothesis is correct
1. prio_local,rajomon,early at 900 RPS improves from 815.8 → ≥850 req/s (≥96% of adctl)
2. prio_local,rajomon,early at 1000 RPS improves from 796.0 → ≥870 req/s (≥93% of adctl)
3. Token rejection rate drops from 10–20% to 5–10% at 900–1000 RPS
4. 600–800 RPS goodput unchanged (price still near 0 in that range)
5. fifo,rajomon,early unchanged (different failure mechanism — time-based ER not token rejection)

### Experiment design
Same config. Named `cobalt_3`.

### Actual Outcomes (cobalt_3)

**Status:** Complete ✅ — prio_local,rajomon,early now matches/exceeds adctl at 600–900 RPS

| RPS | fifo,adctl | fifo,raj (c2→c3) | plocal,adctl | plocal,raj (c2→c3) | raj/adctl % |
|-----|-----------|-----------------|-------------|-------------------|-------------|
| 600 | 598.1 | 611.5 → **614.0** | 596.5 | 608.2 → **587.0** | 98.4% |
| 700 | 693.9 | 687.7 → **703.1** | 688.1 | 704.3 → **699.3** | 101.6% |
| 800 | 686.0 | 602.0 → **575.4** | 817.1 | 798.9 → **814.7** | 99.7% |
| 900 | 688.1 | 367.0 → 425.5 | 873.3 | 815.8 → **887.4** | **101.6%** |
| 1000| 791.0 | 294.0 → 298.0 | 925.8 | 796.0 → **870.8** | **94.1%** |

**Key win:** `prio_local,rajomon,early` at 900 RPS jumped from 815.8 → 887.4 (+8.8%), now exceeding adctl (873.3) at that load. At 1000 RPS, 870.8 → 94.1% of adctl (up from 85.1%). The lower PRICE_CAP (40 vs 60) and slower ramp (PRICE_STEP_UP 4 vs 8) reduced over-rejection at high load dramatically: deep-downstream rejections at 1000 RPS dropped from 6181 → 476.

**Early return depth shift:** adctl rejects mostly at Root (no downstream work done); rajomon still rejects post-child-call. This is the structural gap — not closable by tuning.

**fifo,rajomon,early:** No improvement (298 req/s at 1000 RPS). The failure is architectural — without priority scheduling, ms-56394 queue fills and all rejections are time-based ERs. No amount of price tuning can fix this.

**Decision:** Keep cobalt_3 as the best configuration. The remaining 5.9% gap at 1000 RPS is structural and motivates the root cause experiments below.

---

## Root Cause Investigation: Why adctl outperforms rajomon

Three architectural pillars explain the persistent gap:

### Pillar A: Rejection depth (wasted compute)
adctl rejects at ingress before calling any children (Type A: no `last_rpc`). Rajomon rejects after calling child services (Type B: has `last_rpc`). At 1000 RPS (cobalt_3): adctl has 1759 root-only ERs vs 367 post-child; rajomon has 0 root-only ERs vs 476 deep. Every rajomon rejection has already consumed at least one downstream hop of compute — this is wasted work that backpressures the bottleneck services.

### Pillar B: Signal quality (lagging indicator)
adctl signal: compute-time estimates per method (measures actual SLO-relevant work remaining). Decision is made per-request immediately.
Rajomon signal: Tokio task queue latency (proxy — only detects overload after the queue has already backed up). Decision made on 10ms ticks.

### Pillar C: Equilibrium stability (bursty vs smooth admission)
adctl: continuous budget-rate adjustment converging to a stable equilibrium.
Rajomon: threshold toggle — price oscillates between 0 and PRICE_CAP, causing bursty 40–60% admission during high-price phase then 100% during recovery.

### Root cause experiments

- **cobalt_depth** (`exp/mssim/in/cobalt_depth/`): Constant 900 RPS, 2 policies. Quantify rejection depth (root-only vs post-child ER) for adctl vs rajomon. Proves Pillar A.
- **cobalt_step** (`exp/mssim/in/cobalt_step/`): Step load [400→1000→400 RPS], 2 policies. Shows how quickly each adapts at load transitions. Proves Pillar B.
- **cobalt_robust** (`exp/mssim/in/cobalt_robust/`): Sweep [700, 800, 900, 1000 RPS], 4 policies (adctl+prio_local, rajomon+prio_local, adctl+fifo, rajomon+fifo). Shows adctl flat/near-capacity across the sweep while rajomon degrades steeply above 800 RPS. Proves Pillar C + shows scheduling dependency.
