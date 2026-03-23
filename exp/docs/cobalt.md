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
