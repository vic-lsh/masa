# Empirical Admission Control via P(complete | time_left)

## Motivation

The GROVE track ran five iterations attempting to reduce early-return over-shedding in
`prio_local,est_mean_var,early` for Hotel at SLO=50ms. The best result (grove_5) reduced
Reservation ER from 295/s to 173/s at 1400 RPS and improved goodput by +122 over the
original baseline, but left a persistent ~44 goodput gap vs `prio_local,early` (ple).

Every GROVE iteration was a variant of the same approach: improve the estimate of future
remaining computation time (`est_remaining`) and use it as an early-return threshold. The
fundamental problem is that `est_remaining` accumulates queue delay alongside actual
computation time. Under overload, queue delay dominates the observed `post_child_duration`
and inflates `est_remaining`, causing preemptive ER on requests that could have completed.
Attempts to debias the estimate (asymmetric α, floor EMA) partially suppressed inflation
but couldn't cleanly separate the two components.

**The root problem is the estimation approach, not the parameters.** Queue delay is a
property of the system's current load, not the request's inherent computation cost. Any
EMA-based estimator that observes total post-child latency will conflate the two.

This document proposes a different approach: eliminate the computation time estimate
entirely and replace the ER decision with an empirical measurement of the outcome we
actually care about.

---

## Core idea

At the early-return decision point (`before_child_rpc`), the request has `time_left`
remaining in its SLO budget. The question is: **will this request complete within SLO if
admitted?**

Rather than estimating remaining computation time and comparing it to `time_left`, observe
empirically: of all past requests that had approximately `time_left` remaining at this
decision point, what fraction actually completed within SLO?

Call this `P(complete | api_type, time_left)`. Shed the current request with probability
`1 - P`, subject to a minimum admission rate (probe floor).

This is a direct measurement of the outcome we care about. Queue delays, computation time,
priority scheduling, and inter-hop wait times all affect whether a request completes —
but they're integrated out by the observation. No estimation of their individual
contributions is required.

---

## Design

### Data structure

Maintain a completion rate map per (api_type, time_left_bucket):

```
CompletionRateMap:
    inner: HashMap<(Api, usize), f64>   // p ∈ [0.0, 1.0]
```

Shared across all requests on the same server, behind an `Arc<Mutex<...>>` like the
existing `LatencyMap`. Initialized to `p = 1.0` for all buckets (admit everything at
startup, learn from observations).

**Time_left buckets** (for SLO=50ms; bucket boundaries scale with SLO):
- Bucket 0: `time_left < 5ms`  — almost certainly doomed
- Bucket 1: `time_left ∈ [5ms, 10ms)`
- Bucket 2: `time_left ∈ [10ms, 20ms)`
- Bucket 3: `time_left ∈ [20ms, 35ms)`
- Bucket 4: `time_left ∈ [35ms, 50ms)`
- Bucket 5: `time_left ≥ 50ms` — full budget, never shed

Logarithmic spacing towards zero captures where the interesting decisions happen.
Bucket boundaries could be expressed as fractions of the SLO to generalize across
applications with different SLOs.

### EMA update

When an outcome is observed (request completed or failed):

```
let outcome = if completed_within_slo { 1.0 } else { 0.0 };
p = p + α * (outcome - p);
```

α controls adaptation speed. Separate α values for rising (more completions than
expected) and falling (fewer completions) may help — biasing toward fast response
to deteriorating conditions and slower recovery, to avoid oscillation.

### ER decision (before_child_rpc)

```
let time_left = ctx.e2e_deadline().saturating_sub(time_now());
let bucket = time_left_to_bucket(time_left, ctx.slo());
let p = completion_rate_map.get(ctx.api(), bucket);
let p_admit = p.max(PROBE_FLOOR);   // e.g., PROBE_FLOOR = 0.05

let rand = pseudo_random(ctx.request_id(), resolved_method_id, hop_index);
if EARLY_RETURN && rand > p_admit {
    return Err(self.early_return.issue_error());
}
```

The pseudo-random value is derived deterministically from the request ID and hop
index (as in grove_3), avoiding the need for a thread-local RNG while still providing
uniform-ish admission decisions.

### Outcome observation

The ER decision is made mid-request. The outcome is known only at completion. The
per-request `ParentContext` (already tracks per-request state in local.rs) records:

```
first_er_decision: Option<(Api, usize)>   // (api_type, bucket) at first ER check
is_probe: bool                             // admitted via probe floor despite low p
```

At request completion, the stored bucket is used to update `CompletionRateMap`. Only
the first ER decision point's bucket is updated per request — attribution is cleanest
at the earliest decision point, since that is the one with the most leverage.

**What counts as "completed"?** The request returned a successful response before
`ctx.e2e_deadline()`. This is checkable in the response path. Failed ER (the request
ERed a downstream hop later) counts as incomplete.

### Probe floor

Always admit at least `PROBE_FLOOR` (5%) of requests in every bucket, regardless of p.
This ensures:
1. Observation stream is maintained in every bucket even under heavy shedding
2. The system can detect capacity recovery without waiting for external load change
3. Avoids the hysteresis problem where shedding all requests in a bucket causes p to
   stagnate at a stale value

The probe floor requests are marked `is_probe = true` in the per-request state. Their
outcomes are counted the same as non-probe outcomes — a probe that completes is still
a successful completion.

### Interaction with existing mechanisms

**Child deadline propagation:** `est_remaining` is still used to set the deadline passed
to child RPCs (`child_deadline = ctx.deadline() - est_remaining`). This urgency
tightening is orthogonal to the ER decision and should remain. Only the ER threshold
check is replaced by empirical P.

**EDF reprioritization (before_poll):** Unchanged. Before_poll uses `ctx.deadline()` for
EDF ordering — this is unaffected by the admission mechanism.

**0-injection:** When a request ERs a child hop, the existing 0-injection into
`est_after_child_latency` still applies (since est_remaining is still used for child
deadline computation). The empirical P mechanism has its own feedback loop through
outcome observations.

---

## Potential issues

### 1. Sample starvation in small-time_left buckets at low load

At 100 RPS, requests arrive with large SLO budgets and complete quickly. Almost no
requests enter the small-time_left buckets (0–10ms) at the ER decision point. Those
buckets stay at their initialized value of p=1.0.

When load suddenly increases and requests do start appearing in those buckets, the
estimate is stale (p=1.0 → no shedding), causing a warmup period where shedding is
too conservative. This would show up as a burst of missed SLOs at RPS step transitions.

**Mitigation options:**
- Initialize buckets at p=0.5 rather than p=1.0 (more conservative prior)
- Require a minimum sample count before trusting the estimate; use conservative
  fallback (p=0.5 or fall back to est_remaining check) until N observations accumulated
- Use the grove_5 est_remaining mechanism as a fallback when a bucket has too few
  samples (hybrid approach)

### 2. Delayed feedback and attribution error

The outcome is observed 20–40ms after the ER decision. During that window, load
conditions may have changed — especially at RPS step transitions. An outcome
observed during the 1400 RPS step was set in motion at the 1200 RPS step boundary.

This is an inherent limitation of any outcome-based feedback. The EMA's temporal
smoothing partially absorbs this. Longer `DurationSecs` per RPS step reduces the
fraction of outcomes that span step boundaries.

In production (gradual load changes), this is less of a concern than in experiments
with sharp step transitions.

### 3. Multiple ER decision points per request

A Reservation request calls ~5 downstream services. `before_child_rpc` fires at each
hop with decreasing `time_left`. Attribution only goes to the first decision point.
Intermediate hops (where the request passed but eventually failed later) don't update
their respective buckets.

This means intermediate-bucket estimates are informed only by requests that were
admitted at that bucket AND made their final ER decision at that bucket. Requests
that passed through earlier could have been counted but aren't. This reduces sample
efficiency.

**Alternative:** Update the completion rate for the bucket at the FINAL ER decision
point before completion — this would give each bucket information about the "last
chance" to ER. Or update all buckets the request passed through. The tradeoff is
attribution noise vs sample efficiency.

### 4. Interaction between probe floor and system load

Probes admitted in heavily overloaded buckets (p≈0, small time_left) will mostly
fail, correctly pushing p toward 0. But they also consume real CPU and contribute to
queue pressure. If the probe floor is too high (say 20%), the probe traffic alone
could prevent the system from recovering by keeping queues full.

At 5%, with ~700 Reservation/s at 1400 RPS, probe traffic in a bucket that is being
heavily shed is at most 0.05 × 700 = 35 requests/s — a small fraction of total load.

### 5. Adaptation rate (α) choice

The EMA α controls how fast the completion rate responds to load changes. Too large
(α=0.5): noisy, oscillates under stable load. Too small (α=0.01): slow to respond
at step transitions, defeated by the sample starvation problem.

Unlike est_remaining where the α tuning space was well-explored (FLINT track),
there's no prior tuning data for completion rate EMAs. The right α depends on
observation rate per bucket, which depends on load. An adaptive α (scale with
sample rate) might be preferable.

### 6. Survivorship bias and priority interaction

High-priority requests (tight deadlines) benefit from EDF scheduling — they get polled
sooner after becoming ready. So `P(complete | time_left=small)` is partly elevated by
the fact that small-time_left requests have high priority and get favorable scheduling.

This is actually the correct behavior: the empirical P captures the realized completion
rate given the whole policy (scheduling + ER). If priority scheduling helps urgent
requests complete, that should be reflected in P and result in less aggressive shedding
of those requests.

However, it means P estimates are policy-coupled: if the scheduling policy changes,
learned P values are temporarily invalid. The EMA adapts, but there's a transient
period of misestimation.

### 7. SLO heterogeneity

Hotel has two APIs (Search, Reservation) with different SLOs in some configurations
(e.g., different SLO values in gen_config). The time_left buckets should be
SLO-normalized (expressed as fractions of the SLO) rather than absolute milliseconds
to allow the same bucket structure to apply across APIs with different SLOs.

For the pine_10_lowslo config (both APIs at 50ms SLO), this distinction doesn't
matter — but it matters for generalization to other apps (e.g., Socialnet with
shorter SLOs, mssim with different trace SLOs).

---

## Evaluation plan

### Baseline

grove_5 on `pine_10_lowslo` (Hotel, SLO=50ms, RPS=[100,400,800,1200,1400,1600,1800,2000]).
Key reference points:
- emv goodput at 1400: 523.8 (vs ple 567.8, gap −44)
- Reservation ER at 1400: 173/s (vs ple 130/s, gap 43/s)

The new mechanism needs to reduce Reservation ER at 1400 from 173 toward ple's 130,
and improve goodput accordingly.

### Experiment config adjustments

**Longer duration per RPS step:** Increase `DurationSecs` from 60 to 120. This gives
bucket estimates more time to converge at each load level before goodput is measured,
and reduces the fraction of outcomes that span step boundaries.

**Larger warmup:** Increase `WarmupSecs` from 20 to 30. More warmup time means the
completion rate map has more observations before measurements begin.

**Consider non-monotonic schedule:** A schedule like [100, 400, 800, 1400, 800, 1400,
2000] tests whether the estimates correctly recover when load drops from 1400 back to
800 — validating the probe floor's feedback maintenance. This would catch if p gets
stuck at near-0 after a heavy-load phase.

### Diagnostic instrumentation

To understand what the mechanism is doing, log at each RPS step:
- Current p value per (api_type, bucket) pair
- Sample count per bucket accumulated so far
- Fraction of admitted requests that were probes (vs normal admission)
- Effective admission rate per bucket

Without these diagnostics, it's hard to distinguish "mechanism worked correctly" from
"mechanism is making wrong decisions but the experiment isn't showing it yet."

### Success criteria

Primary: Reservation ER at 1400 drops below grove_5's 173/s toward ple's 130/s.
Goodput at 1400 rises above grove_5's 523.8.

Secondary: No regression at 1600–2000 RPS vs grove_5. Search continues to be shed
aggressively (expected, since Search P(complete | any time_left) → 0 under load).

Strong success: emv goodput meets or exceeds ple at the majority of overloaded RPS
points. This would be the first time emv consistently matches ple on Hotel.

### What failure would mean

If Reservation ER does not drop despite the new mechanism:
- Check bucket sample counts — starvation may be the issue
- Check p values — if they're not adapting to load, α may be too small or the
  update path has a bug
- Check outcome attribution — completion outcomes may not be reaching the update

If goodput at 1600 regresses while 1400 improves (similar to grove_2 pattern):
- The mechanism may be admitting too many requests at 1400 (p too high) while
  failing to shed enough at 1600 (p too low due to sample starvation during step
  transition)
- Solution: hybrid fallback to est_remaining when bucket p has insufficient samples

---

## Open questions

1. **Bucket boundaries:** Should they be fixed (absolute ms) or SLO-relative
   (fractions of SLO)? SLO-relative is more portable; absolute is simpler to
   implement.

2. **Attribution:** First decision point only, or all decision points the request
   passes through? First is cleaner conceptually; all gives more samples.

3. **Fallback:** When a bucket has fewer than N observations, use est_remaining
   check as fallback (grove_5 behavior) rather than trusting stale p=1.0. What is
   the right N?

4. **α for completion rate EMA:** Start with α=0.1 (same as latency estimator) and
   tune. Consider asymmetric α (faster to decay than to recover) to bias toward
   conservative shedding.

5. **Probe floor magnitude:** 5% is a reasonable starting point. Could be adaptive
   — higher floor when bucket sample count is low (need more observations), lower
   when well-calibrated.

6. **Interaction with grove_5 changes:** Should the empirical admission mechanism
   replace the est_remaining ER check entirely, or layer on top of it? Replacing it
   is cleaner and avoids double-shedding. But keeping est_remaining as a fallback
   for poorly-sampled buckets may improve robustness.
