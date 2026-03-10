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

---
