# PINE — prio_local,est_mean_var,early on Hotel & Socialnet

## Key questions
- Does the goodput advantage of `prio_local,est_mean_var,early` over `prio_oldest,early` observed in mssim translate to real applications (Hotel, Socialnet)?
- Is the current k=0.75, α=0.1 tuning (optimized for the two-trace mssim workload) appropriate for Hotel and Socialnet workloads?
- If a single static policy does not beat `prio_oldest,early` across all apps/loads, what online adaptation mechanism can close the gap?

## Experiment series: pine_1, pine_2, ... (hotel, then socialnet)

### Current code state (start of PINE track)
- `k=0.75, α=0.1` in `LatencyMeanVar::default()` — from CEDAR track (two-trace mssim)
- Branch: `vic/test-across-callgraphs`, commit `f947a975`
- Note: k=1.2 was optimal for single-trace; k=0.75 for two-trace; real apps are a third regime

---

## Phase 0: Baseline sweep — Hotel (pine_1)

**Status:** Running

### Config (exp/hotel/in/pine_1/)
- APIs: Search (SLO=200ms), Reservation (SLO=200ms)
- RPS sweep: 100, 200, 400, 600, 800, 1000, 1400
- WarmupSecs: 20, DurationSecs: 60
- Policies: fifo, prio_oldest,early, prio_local,early, prio_local,est_mean_var,early
- 1 replica per service (baseline; may increase if bottleneck is per-replica, not policy)

### Motivation
Establish where Hotel saturates and how each policy performs. Hotel has a richer call graph than mssim (9 services), so local deadline estimation has more room to differentiate. Key question: does the warm EMA estimator provide better deadline propagation under Hotel's multi-step call graph?

---

## Phase 0: Baseline sweep — Socialnet (pine_2)

**Status:** Pending (after pine_1 completes)

### Config (exp/socialnet/in/pine_2/)
- API: ComposePost (SLO=50ms)
- RPS sweep: 100, 200, 300, 400, 600, 800
- WarmupSecs: 20, DurationSecs: 60
- Policies: fifo, prio_oldest,early, prio_local,early, prio_local,est_mean_var,early
- Note: CLAUDE.md says prio_local "only works for hotel", but ci config includes it for socialnet; we include it to verify

### Motivation
Socialnet has a much deeper call graph (14+ services) and a tighter SLO (50ms vs 200ms). This is a harder workload for local deadline estimation — the tighter SLO leaves less margin for estimation error, and more services means more opportunities for the policy to help (or hurt) via cascaded deadline propagation.

---

## Observed Symptoms (pine_1) — Hotel baseline

**Status:** Complete ✅

### Goodput table (all policies, all RPS)

| RPS  | fifo   | prio_oldest,early | prio_local,early | prio_local,est_mean_var,early |
|------|--------|-------------------|------------------|-------------------------------|
| 100  | 99.83  | 99.82             | 99.82            | 99.83                         |
| 200  | 199.63 | 199.66            | 199.63           | 199.66                        |
| 400  | 399.26 | 399.31            | 399.28           | 399.27                        |
| 600  | 598.91 | 598.93            | 598.96           | 598.89                        |
| 800  | 798.56 | 798.53            | 798.57           | 798.53                        |
| 1000 | 998.08 | 998.31            | 998.24           | 998.24                        |
| 1400 | 167.02 | **1334.86**       | 1276.73          | **1280.95**                   |

System saturates at **1400 RPS** (only differentiation point). Below 1400 all policies deliver near-perfect goodput.

### Delta (prio_local,est_mean_var,early − prio_oldest,early)

**−53.9 RPS at 1400 RPS** (the only load point that matters). prio_local LOSES.

### Root cause

At 1400 RPS, Reservation: prio_oldest=657, prio_local,est_mean_var=594 (−63).
Early-return rates for Reservation: prio_oldest=36.4/s, prio_local,est_mean_var=71.6/s — **2× rate**.
Trigger: `user.User/CheckUser` abandoned at ~65/s under prio_local vs ~36/s for prio_oldest.

k=0.75 propagates overly tight deadlines to CheckUser. Hotel's call paths are fast (p50=5–7ms, SLO=200ms) — large slack that k=0.75 incorrectly treats as urgency via the variance term.

### Other findings
- prio_local,est_mean_var and prio_local,early are nearly identical (gap=4.2 RPS) — mean-var estimator adds no value on Hotel
- fifo collapses to 167 at 1400 — early-return is load-bearing

---

## Observed Symptoms (pine_2) — Socialnet baseline (low-RPS sweep)

**Status:** Incomplete — not in overloaded regime; extended to pine_3

### Key findings
- All policies 100% goodput across 100–800 RPS; no early returns
- p99 ~10ms at 800 RPS vs 50ms SLO — 5× headroom remaining
- Saturation likely at 1500–3000 RPS
- prio_local effectively degrades to prio_global on socialnet (no hardcoded call graph → est_remaining=0 initially)

---

## Observed Symptoms (pine_3) — Socialnet high-RPS sweep

**Status:** Complete ✅

### Goodput table

| RPS  | fifo  | prio_oldest,early | prio_local,early | prio_local,est_mean_var,early |
|------|-------|-------------------|------------------|-------------------------------|
| 500  | 500.0 | 500.0             | 500.0            | 500.0                         |
| 800  | 800.0 | 800.0             | 799.9            | 799.9                         |
| 1200 | 1182.9| 1193.4            | 1190.6           | 1193.9                        |
| 1600 | 931.0 | 1082.3            | **1091.6**       | 1012.2                        |
| 2000 | 336.2 | **1672.3**        | 899.4            | 1354.0                        |
| 2500 | 289.5 | 893.4             | **1002.9**       | 962.3                         |

Saturation onset: ~1400–1500 RPS.

### Delta (prio_local,est_mean_var,early − prio_oldest,early)

| RPS  | Delta   |
|------|---------|
| 1600 | −70.1   |
| 2000 | **−318.3** ← critical overload regime |
| 2500 | +68.9   |

prio_local,est_mean_var LOSES at 2000 RPS (critical), narrowly wins at 2500 RPS.

### Root cause

prio_oldest achieves 1672 at 2000 RPS with ER=327/s. prio_local,est_mean_var achieves 1354 with ER=630/s — more early-returns but less goodput. The policy is shedding completable requests while serving incompletable ones.

Root cause: prio_local has no call graph for socialnet. Despite learning estimates online, ComposePost's parallel-fanout structure (text, user_mention, url_shorten, unique_id called simultaneously) makes per-pair "remaining time" estimates inaccurate. At 2000 RPS, this degrades to worse-than-prio_oldest scheduling. At 2500 RPS (deep overload), estimates converge to saturation values and become reliable enough to win.

---

## Observed Symptoms (pine_4) — Socialnet with k=0 + e2e_deadline ER (Iterations 2+3 combined)

**Status:** Complete ✅ — **prio_local,est_mean_var now beats prio_oldest at all overloaded RPS**

### Goodput table

| RPS  | fifo   | prio_oldest,early | prio_local,early | prio_local,est_mean_var,early |
|------|--------|-------------------|------------------|-------------------------------|
| 500  | 500.0  | 500.0             | 500.0            | 500.0                         |
| 800  | 800.0  | 800.0             | 800.0            | 800.0                         |
| 1200 | 1197.8 | 1149.5            | 1147.3           | **1178.7** (+29.2)            |
| 1600 | 997.9  | 1522.2            | 1485.6           | **1547.1** (+24.9)            |
| 2000 | 1373.3 | 1159.7            | 1211.3           | **1292.9** (+133.2)           |
| 2500 | 332.3  | 1583.3            | **1828.0**       | 1669.5 (+86.2)                |

### Delta (prio_local,est_mean_var,early − prio_oldest,early)

| RPS  | pine_3 (k=0.75) | pine_4 (k=0, e2e ER) | Change |
|------|-----------------|----------------------|--------|
| 1600 | −70.1           | **+24.9**            | +95    |
| 2000 | **−318.3**      | **+133.2**           | +451   |
| 2500 | +68.9           | +86.2                | +17    |

**prio_local,est_mean_var beats prio_oldest at every overloaded RPS in this run.** The massive flip at 2000 RPS (−318 → +133) is the key result.

### ER rates at 2000 RPS
- prio_oldest: 838/s ER → 1159.7 goodput
- prio_local,est_mean_var: 637 (frontend) + 37 (compose_post) = 674/s ER → 1292.9 goodput
- prio_local,early: 787/s ER → 1211.3 goodput

est_mean_var achieves higher goodput with *fewer* early-returns — it's shedding more selectively.

### Note on prio_local,early
prio_local,early wins at 2500 RPS (1828 vs 1669 for est_mean_var). The RMS estimator (used by prio_local,early) apparently outperforms the EMA mean-only (k=0) at deep overload on socialnet. This warrants further investigation in a future iteration (est_mean_var with k>0 or a different adaptive estimator).

### Caveat
High run variance on socialnet — prio_oldest at 2000 dropped from 1672 (pine_3) to 1160 (pine_4). Within-run deltas are more reliable than cross-run absolute values. The sign flip from negative to positive for est_mean_var vs oldest is a real improvement but may need re-verification with repeated runs.

---

## Iteration 1: Reduce k from 0.75 to 0.25 (pine_4 — Hotel)

**Status:** Pending

### Change
Set `k=0.25` in `LatencyMeanVar::default()` (`libs/masa-core/src/latency_estimator/mean_var.rs`).

### Hypothesis
k=0.75 causes over-aggressive early-return on Hotel's Reservation (71.6 ER/s vs 36.4 for prio_oldest). The variance buffer `k*sqrt(var)` adds unnecessary conservatism on fast call paths (p50=5–7ms, SLO=200ms). Reducing to k=0.25 will tighten the buffer, make local deadlines looser, reduce false-positive early-returns, and recover Reservation goodput.

For socialnet: at 2000 RPS the problem is scheduling order (not k), so no improvement expected. At 2500 RPS, reduced downstream ER aggressiveness may help marginally.

### Expected outcomes if hypothesis is correct
1. Hotel 1400 RPS: Reservation ER drops from ~71.6/s toward ~36/s; total goodput improves from 1281 toward 1335+
2. Hotel 1400 RPS: prio_local,est_mean_var matches or beats prio_oldest
3. Socialnet 2000 RPS: No significant change
4. Socialnet 2500 RPS: Slight improvement in est_mean_var

### Experiment design
Run pine_4 for Hotel (identical config to pine_1). After Hotel results confirm/deny, run Socialnet pine_4 (same as pine_3 config).

### Actual Outcomes (pine_4 — Hotel)

**Status:** Complete ✅

| Policy | pine_1 (k=0.75) | pine_4 (k=0.25) | Δ(4−1) |
|---|---|---|---|
| fifo | 167.0 | 350.5 | (run variance) |
| prio_oldest,early | 1334.9 | 1335.5 | +0.6 |
| prio_local,early | 1276.7 | 1354.8 | +78.1 |
| **prio_local,est_mean_var,early** | **1280.9** | **1311.4** | **+30.4** |

Delta vs prio_oldest at 1400 RPS: **−53.9 → −24.1** (gap halved).
Reservation ER: 71.7% → 59.8% (still >> prio_oldest 38.7%).
Decision: gap is closing monotonically; proceed to k=0.

---

## Iteration 2: k=0 (pure mean, no variance term) (pine_5 — Hotel)

**Status:** Pending

### Change
Set `k=0.0` in `LatencyMeanVar::default()`.

### Hypothesis
Variance term `k*sqrt(var)` is the source of over-estimation — under overload, variance inflates as service times become erratic. Removing it entirely (k=0, pure mean) will minimize deadline tightening, reduce false-positive Reservation ERs, and close the remaining −24 RPS gap.

Risk: if mean itself is queue-inflated, k=0 still over-sheds. If too relaxed, wasted resources on incompletable requests would show as Search goodput drop.

### Expected outcomes
1. Hotel 1400 RPS: Reservation ER further reduced (from 59.8% toward 38.7%)
2. Total goodput improved from 1311 toward 1335+
3. prio_local,est_mean_var,early approaches or exceeds prio_oldest,early

### Experiment design
Run pine_5 (Hotel, same config as pine_1/pine_4).

---

### Actual Outcomes (pine_5 — Hotel, k=0)

**Status:** Complete ✅

k=0 trend: pine_1(−53.9) → pine_4(−24.1) → pine_5(**−16.0**). Reservation ER: 71.7% → 59.8% → **47.4%** (vs prio_oldest 27.5%).

Gap is closing but converging non-zero: even at k=0, mean inflation from queuing still over-sheds Reservation. Pure k-tuning has hit diminishing returns; different approach needed.

Decision: implement Iteration 3 (separate ER from priority via e2e_deadline).

---

## Iteration 3: Use e2e_deadline for ER, preserve local deadline for priority (pine_6 — Hotel)

**Status:** Pending

### Change
`EarlyReturnHandler::check()` and local policy's `before_child_rpc` ER check now use `ctx.e2e_deadline()` (gateway_entry + slo = actual SLO boundary) instead of `ctx.deadline()` (tightened local deadline). The tightened local deadline is preserved for reprioritization (before_poll) and scheduling priority (prio_hint).

### Hypothesis
Root cause of over-shedding: ER fires at the per-hop tightened deadline rather than the actual SLO. The local policy sets child deadline = parent_deadline - est_remaining for scheduling purposes, but this tightened deadline should not trigger ER (which should only fire when the request will definitely miss the SLO). By anchoring ER to e2e_deadline while keeping tightened deadline for EDF scheduling, we eliminate false-positive ERs entirely. prio_local's ER behavior becomes identical to prio_oldest (fire only at actual SLO), while retaining EDF priority ordering as a differentiating advantage.

### Expected outcomes
1. Hotel 1400 RPS: Reservation ER drops to ~27.5% (matching prio_oldest)
2. prio_local,est_mean_var achieves ≥ prio_oldest goodput (EDF ordering benefit appears)
3. Socialnet: similar improvement — fewer false-positive ERs at 2000 RPS

### Actual Outcomes (pine_6 — Hotel, k=0, e2e_deadline ER)

**Status:** Complete ✅ (slight regression vs pine_5)

| Policy | pine_5 (k=0) | pine_6 (k=0, e2e ER) | Δ(6−5) |
|---|---|---|---|
| prio_oldest,early | 1340.3 | 1340.6 | +0.3 |
| **prio_local,est_mean_var,early** | **1324.3** | **1321.2** | **−3.1** |

Delta vs prio_oldest: pine_5 = **−16.0** → pine_6 = **−19.4** (slight regression).

### Root cause

The e2e_deadline ER fix was not sufficient. Reservation ER remains high (~46.8/s vs prio_oldest ~22.4/s). The ER fix moved the threshold to the actual SLO boundary, but `before_poll` still reprioritizes using `ctx.deadline()` (the *tightened* local deadline). When the tightened deadline expires before the actual SLO:
- `remaining = ctx.deadline() - now = saturating_sub = 0`
- `PriorityHint::new(0)` = lowest possible priority
- Task gets stuck at the bottom of the queue
- Request eventually misses the actual SLO even though it could have completed in time

This is a **priority inversion bug**: the local deadline tightening, intended to improve scheduling order, causes the opposite effect once the tightened deadline passes. Fixing ER is not enough — `before_poll` must also use `e2e_deadline` for reprioritization.

---

## Iteration 4: Use e2e_deadline for reprioritization in before_poll (pine_7 — Hotel)

**Status:** Pending

### Change
`before_poll` in `local/local.rs` now reprioritizes using `ctx.e2e_deadline()` instead of `ctx.deadline()`. The tightened local deadline is still used for child RPC priority propagation (`before_child_rpc` prio_hint), but the task's own scheduling priority is anchored to the actual SLO.

### Hypothesis
The priority inversion bug: when the tightened local deadline expires (before actual SLO), `ctx.deadline() - now = 0`, demoting the task to the lowest priority. This causes Reservation requests to be starved in the queue and eventually miss the SLO even though they could complete. By using `e2e_deadline` for reprioritization, the task retains urgency relative to the actual SLO deadline, avoiding false starvation.

Combined with the Iteration 3 fix (ER uses e2e_deadline), this should eliminate both false-positive early-returns *and* priority inversions. prio_local's behavior should approach prio_oldest's ER rate while retaining EDF priority ordering as a differentiated advantage.

### Expected outcomes
1. Hotel 1400 RPS: Reservation ER drops to ~22–28/s (toward prio_oldest's 22.4/s)
2. prio_local,est_mean_var goodput ≥ prio_oldest (1340+)
3. The prio_local benefit emerges: EDF ordering should improve throughput *above* prio_oldest

### Experiment design
Run pine_7 for Hotel (identical config to pine_1/pine_4/pine_5/pine_6). This directly tests whether the priority inversion fix eliminates the remaining gap.

---
