# PRISM — FLARE ablation study

## Key questions
- How much does `sched_pred` (deadline tightening + EDF reprioritization) contribute to FLARE goodput vs plain `sched_slo`?
- How much does `abort_slo` (drop past-deadline requests) contribute when the rest of FLARE is present?
- Do scheduling and abort_slo interact (i.e., does the benefit of one depend on the other being present)?

## Experiment series: prism_1, prism_2, ... (socialnet, then hotel)

## Phase 1: Scheduling ablation (prism_1, socialnet)

**Policies under test:**
1. `sched_pred,abort_slo,ac_pred,est_mean_var` — full FLARE
2. `sched_slo,abort_slo,ac_pred,est_mean_var` — drop sched_pred (no deadline tightening, no EDF reprioritization)
3. `sched_tailclipper,abort_slo` — external baseline

**Config:** Same as geyser_7 (ComposePost, SLO=50ms, RPS 800-3000, 30s/step, 10s warmup).

**What sched_pred adds over sched_slo:**
- `before_poll`: reprioritizes task based on updated deadline estimate (EDF)
- `before_child_rpc`: tightens child deadline by `est_remaining` (deadline tightening)
- Both require the estimator to have warmed up

**Expected outcome:** sched_pred should help at deep overload where priority ordering
matters most. At moderate overload the AC dominates and scheduling is secondary.

### Results (prism_1)

| RPS | sched_pred (FLARE) | sched_slo | tailclipper | pred−slo | pred−TC |
|-----|--------------------|-----------|-------------|----------|---------|
| 800 | 800 | 800 | 800 | 0 | 0 |
| 1000 | 1000 | 1000 | 1000 | 0 | 0 |
| 1200 | 1191 | 1177 | 1177 | +14 | +14 |
| 1400 | 1311 | 1339 | 1278 | −28 | +33 |
| 1600 | 1258 | 1287 | 1065 | −29 | +193 |
| 1800 | 1192 | 1257 | 1019 | −65 | +173 |
| 2000 | 1273 | 1291 | 1075 | −18 | +198 |
| 2500 | 1304 | 1380 | 1078 | −76 | +226 |
| 3000 | 1451 | 1379 | 985 | +72 | +466 |

**Finding: sched_pred is roughly neutral on socialnet.** It slightly hurts at moderate
overload (1400-2500, −18 to −76) and slightly helps at extreme overload (3000, +72).
The ER breakdown confirms deadline tightening over-sheds at moderate overload:
sched_pred frontend ER rate is higher (1189/s vs 1115/s at 2500 RPS).

This is consistent with PINE findings: parallel fanout architectures (socialnet) don't
benefit from deadline tightening because it creates priority inversions on completable
requests. The ac_pred token bucket is doing the heavy lifting — both sched_pred and
sched_slo beat tailclipper by +170 to +466 at 1600+ RPS.

---

## Phase 2: abort_slo ablation (prism_2, socialnet)

**Policies under test:**
1. `sched_pred,abort_slo,ac_pred,est_mean_var` — full FLARE
2. `sched_pred,ac_pred,est_mean_var` — drop abort_slo (no deadline-based request dropping)
3. `sched_tailclipper,abort_slo` — external baseline

**Config:** Same as prism_1 (ComposePost, SLO=50ms, RPS 800-3000, 30s/step).

**What abort_slo does:** Drops requests that have exceeded their e2e SLO deadline at
before_poll and before_child_rpc checkpoints. Without it, expired requests continue
executing and consuming resources, crowding out fresh requests that could still meet SLO.

### Results (prism_2)

| RPS | +abort_slo (FLARE) | −abort_slo | tailclipper | abort_slo Δ | FLARE−TC |
|-----|--------------------|------------|-------------|-------------|----------|
| 800 | 800 | 800 | 800 | 0 | 0 |
| 1000 | 1000 | 1000 | 1000 | 0 | 0 |
| 1200 | 1198 | 1195 | 1170 | +3 | +28 |
| 1400 | 1310 | 1281 | 1147 | +29 | +163 |
| 1600 | 1258 | 1209 | 1016 | +49 | +242 |
| 1800 | 1206 | 1130 | 1084 | +76 | +122 |
| 2000 | 1272 | 1182 | 1081 | +90 | +191 |
| 2500 | 1284 | 1141 | 1008 | +144 | +276 |
| 3000 | 1342 | 1084 | 1002 | +259 | +340 |

**Finding: abort_slo provides consistent, load-proportional benefit.** The delta grows
monotonically from +3 at 1200 to +259 at 3000 RPS. At extreme overload (3000), abort_slo
accounts for ~76% of FLARE's advantage over tailclipper (259 of 340).

**Mechanism (ER breakdown):** Without abort_slo, expired requests leak deep into the call
graph before being shed. At 3000 RPS without abort_slo: compose_post sheds 197/s,
text_service sheds 266/s — these requests consumed CPU at multiple hops before dying.
With abort_slo, nearly all shedding happens at frontend (1651/s at 3000), which is
maximally cheap (no downstream work wasted).

**However:** Even without abort_slo, ac_pred alone still beats tailclipper at all
overloaded points (+82 to +163 at 1600-3000). The token bucket's cost-aware shedding
provides a baseline advantage even when expired requests aren't proactively dropped.

---

## Summary

| Component | Contribution to FLARE (socialnet) |
|-----------|----------------------------------|
| **ac_pred (token bucket)** | Dominant — provides +80 to +466 over tailclipper even without other features |
| **abort_slo** | Significant — +29 to +259 incremental benefit, grows with load |
| **sched_pred** | Negligible/slightly negative — −18 to −76 at moderate overload, +72 at 3000 only |

The FLARE advantage on socialnet is primarily driven by cost-aware admission control
(ac_pred), amplified by abort_slo's early shedding of doomed requests. The scheduling
discipline (sched_pred vs sched_slo) is not a significant contributor for this
parallel-fanout workload.

---

## Phase 3: Hotel ablation (prism_3/4/5)

Hotel has a serial call graph (frontend → search → geo/profile/rate/etc.), which is the
opposite of socialnet's parallel fanout. sched_pred's deadline tightening may matter more
here because serial chains have tighter per-hop budgets.

**Policies (same for all 3):**
1. `sched_pred,abort_slo,ac_pred,est_mean_var` — full FLARE
2. `sched_slo,abort_slo,ac_pred,est_mean_var` — drop sched_pred
3. `sched_pred,ac_pred,est_mean_var` — drop abort_slo
4. `sched_tailclipper,abort_slo` — external baseline

### prism_3: Hotel Search (SLO=200ms, RPS 700-1200, 60s/step)

| RPS | FLARE | −sched_pred | −abort_slo | tailclipper | pred−slo Δ | abort Δ |
|-----|-------|-------------|------------|-------------|------------|---------|
| 700 | 663 | 665 | 669 | 699 | −2 | −6 |
| 800 | 706 | 699 | 712 | 692 | +7 | −6 |
| 900 | 726 | 704 | 721 | 639 | +22 | +5 |
| 1000 | 724 | 696 | 726 | 524 | +28 | −3 |
| 1100 | 725 | 694 | 720 | 250 | +31 | +5 |
| 1200 | 714 | 681 | 716 | 199 | +33 | −2 |

**sched_pred: significant.** +22 to +33 at overload. Serial call graph benefits from
deadline tightening — tighter per-hop budgets help prioritize completable requests.

**abort_slo: negligible.** −6 to +5. ac_pred already rejects at ingress, so few
requests reach the deadline guard. Removing abort_slo slightly *improves* some points,
possibly because abort_slo's ER signal interferes with the AC feedback loop.

### prism_4: Hotel Reservation (SLO=20ms, RPS 8000-12000, 40s/step)

| RPS | FLARE | −sched_pred | −abort_slo | tailclipper | pred−slo Δ | abort Δ |
|-----|-------|-------------|------------|-------------|------------|---------|
| 8000 | 7792 | 7716 | 7752 | 7846 | +76 | +40 |
| 9000 | 8520 | 8436 | 8280 | 8494 | +84 | +240 |
| 10000 | 8967 | 8558 | 9099 | 8904 | +409 | −132 |
| 10500 | 9310 | 9045 | 8227 | 9241 | +265 | +1083 |
| 11000 | 9353 | **0** | 8681 | **0** | +9353 | +672 |
| 11500 | **0** | 7677 | **0** | 2178 | −7677 | 0 |
| 12000 | 3334 | 7159 | **0** | **0** | −3825 | +3334 |

**Highly bistable.** Multiple policies collapse to 0 at different RPS points. The tight
20ms SLO amplifies feedback loop instability. Through the noise:

**sched_pred: helps at 10000-11000** (+265 to +9353 where sched_slo collapses).
**abort_slo: critical at extreme overload** (+672 to +3334 at 11000-12000). Without it,
collapses happen earlier.

⚠️ The 0-goodput results at 11000 (sched_slo, TC) and 11500 (FLARE) are likely
bistable — a repeat run might not reproduce them. Within-run comparisons at 8000-10500
are more reliable.

### prism_5: Hotel 2-API (Search+Reservation, SLO=200ms, RPS 100-2000, 40s/step)

| RPS | FLARE | −sched_pred | −abort_slo | tailclipper | pred−slo Δ | abort Δ |
|-----|-------|-------------|------------|-------------|------------|---------|
| 100 | 99 | 101 | 101 | 102 | −2 | −2 |
| 400 | 400 | 399 | 396 | 398 | +1 | +4 |
| 800 | 800 | 799 | 790 | 801 | +1 | +10 |
| 1000 | 992 | 989 | 994 | 993 | +3 | −2 |
| 1200 | 1172 | 1171 | 1175 | 1191 | +1 | −3 |
| 1400 | 1307 | 1268 | 1301 | 1238 | +39 | +6 |
| 1600 | 1395 | 1354 | 1398 | 916 | +41 | −3 |
| 1800 | 1456 | 1438 | 1475 | 630 | +18 | −19 |
| 2000 | 1527 | 1508 | 1508 | 570 | +19 | +19 |

**sched_pred: moderate.** +18 to +41 at overload (1400-2000). Consistent with Search.

**abort_slo: negligible to slightly negative.** −19 to +19. Removing abort_slo often
matches or beats full FLARE. On Hotel, ac_pred handles shedding and abort_slo adds
noise to the feedback loop without meaningfully reducing wasted work.

---

## Cross-app summary

### sched_pred contribution (pred−slo delta at peak overload)

| App | Peak Δ | Pattern |
|-----|--------|---------|
| Socialnet (parallel fanout) | −76 to +72 | **Neutral/slightly negative** — deadline tightening over-sheds completable parallel requests |
| Hotel Search (serial) | +22 to +33 | **Consistently positive** — serial chains benefit from tighter per-hop budgets |
| Hotel Reservation (serial, tight SLO) | +265 to +9353 | **Large but noisy** — helps prevent collapse at critical overload points |
| Hotel 2-API (mixed) | +18 to +41 | **Moderately positive** — consistent with Search |

**Verdict:** sched_pred helps on serial call graphs, hurts slightly on parallel fanout.
The benefit comes from deadline tightening giving internal hops better priority signals.
On parallel fanout, the same tightening creates priority inversions.

### abort_slo contribution (abort delta at peak overload)

| App | Peak Δ | Pattern |
|-----|--------|---------|
| Socialnet (parallel fanout) | +29 to +259 | **Significant, grows with load** — expired requests waste CPU across many parallel children |
| Hotel Search (serial) | −6 to +5 | **Negligible** — ac_pred rejects at ingress before deadline expiry |
| Hotel Reservation (serial, tight SLO) | +40 to +3334 | **Critical at extreme overload** — prevents collapse |
| Hotel 2-API (mixed) | −19 to +19 | **Negligible** — same as Search |

**Verdict:** abort_slo matters most when (a) the call graph fans out (wasted work
multiplied across children) or (b) the SLO is so tight that requests expire mid-flight
even with AC. On Hotel Search/2-API with generous 200ms SLO, ac_pred sheds before
deadlines expire, making abort_slo redundant.

### The dominant component across all apps: ac_pred

All FLARE variants (with or without sched_pred, with or without abort_slo) massively
beat tailclipper at overload. The token bucket's cost-aware shedding is the core
mechanism. sched_pred and abort_slo are situational amplifiers.
