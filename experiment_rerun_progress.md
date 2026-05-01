# Branch Rerun Progress

Started: 2026-05-01 UTC

Branch: `codex/mssim-conditional-variants`

Target policy: `sched_pred,abort_slack,ac_pred,est_mean_var`

Goal: duplicate the requested experiments, preserve old data, and try up to 5 optimization rounds per experiment to beat the prior match/original result for the target policy.

Requested order:

1. `exp/mssim/plots/cg8651_acCmp`
2. `exp/mssim/plots/s1467_acCmp`
3. `exp/mssim/plots/s3204_acCmp`
4. `exp/hotel/in/test1`

## Setup Notes

- Original experiment inputs and outputs are left untouched.
- Duplicated run names use the `branch_rerun_20260501` suffix.
- MSSIM duplicates use `trace-analysis/graph_branch/...` call graph directories so this branch's conditional-variant graph data is exercised.
- Optimization rounds are capped at 5 per experiment.

## Progress

- [x] Prepare duplicated inputs.
- [x] Run and evaluate `cg8651_acCmp`.
- [x] Run and evaluate `s1467_acCmp`.
- [x] Run and evaluate `s3204_acCmp`.
- [ ] Run and evaluate `hotel/test1`.

## Round Matrix

All rounds use only `sched_pred,abort_slack,ac_pred,est_mean_var`.

| Round | `aimd_er_threshold` | `fanout_min_samples` | `fanout_deadline_legacy_fraction` | `fanout_root_only` | Notes |
| --- | ---: | ---: | ---: | --- | --- |
| 1 | 0.10 | 3 | 0.0 | false | Latest branch default plus explicit full pred params. |
| 2 | 0.28 | 3 | 0.0 | false | Prior cg8651 probe threshold. |
| 3 | 0.40 | 3 | 0.0 | false | Ember-style threshold anchor. |
| 4 | 0.40 | 1 | 0.0 | false | Let sparse exact fanout signatures participate sooner. |
| 5 | 0.40 | 3 | 1.0 | true | Conservative fanout deadline behavior, root-only. |

## Run Log

### 1. `cg8651_acCmp`

- Baseline source: `exp/mssim/plots/cg8651_acCmp`
- Duplicates: `exp/mssim/in/cg8651_acCmp_branch_rerun_20260501_r{1..5}`
- MSSIM call graph: `trace-analysis/graph_branch/S_86516878`
- Round 1: `cg8651_acCmp_branch_rerun_20260501_r1`
  - Status: completed successfully.
  - Output: `exp/mssim/out/cg8651_acCmp_branch_rerun_20260501_r1`
  - Plots: `exp/mssim/plots/cg8651_acCmp_branch_rerun_20260501_r1`
  - Sum goodput, target policy: 9226.534
  - Original target-policy sum: 9065.434 (`+161.100`)
  - Original competing-policy match sum: 7026.268 (`+2200.266`)
  - Decision: stop after round 1 because it beats both the original target-policy run and the competing-policy match.

### 2. `s1467_acCmp`

- Baseline source: `exp/mssim/plots/s1467_acCmp`
- Duplicates: `exp/mssim/in/s1467_acCmp_branch_rerun_20260501_r{1..5}`
- MSSIM call graph: `trace-analysis/graph_branch/S_14677443`
- Round 1: `s1467_acCmp_branch_rerun_20260501_r1`
  - Status: completed successfully.
  - Sum goodput, target policy: 4392.066
  - Original target-policy sum: 4414.034 (`-21.968`)
  - Original competing-policy match sum: 4139.933 (`+252.133`)
  - Decision: continue; this beats the match but not the original target-policy run.
- Round 2: `s1467_acCmp_branch_rerun_20260501_r2`
  - Status: completed successfully.
  - Sum goodput, target policy: 4403.966
  - Original target-policy sum: 4414.034 (`-10.068`)
  - Original competing-policy match sum: 4139.933 (`+264.033`)
  - Decision: continue; closer, but still below original target-policy aggregate.
- Round 3: `s1467_acCmp_branch_rerun_20260501_r3`
  - Status: completed successfully.
  - Sum goodput, target policy: 4368.301
  - Original target-policy sum: 4414.034 (`-45.733`)
  - Original competing-policy match sum: 4139.933 (`+228.368`)
  - Decision: continue; threshold 0.40 regressed aggregate goodput.
- Round 4: `s1467_acCmp_branch_rerun_20260501_r4`
  - Status: completed successfully.
  - Sum goodput, target policy: 4322.033
  - Original target-policy sum: 4414.034 (`-92.001`)
  - Original competing-policy match sum: 4139.933 (`+182.100`)
  - Decision: continue to the fifth and final allowed round; `fanout_min_samples=1` regressed.
- Round 5: `s1467_acCmp_branch_rerun_20260501_r5`
  - Status: completed successfully.
  - Sum goodput, target policy: 4316.434
  - Original target-policy sum: 4414.034 (`-97.600`)
  - Original competing-policy match sum: 4139.933 (`+176.501`)
  - Decision: stop at the five-round cap. Best is round 2: 4403.966, which beats the match but is 10.068 below the original target-policy run.

### 3. `s3204_acCmp`

- Baseline source: `exp/mssim/plots/s3204_acCmp`
- Duplicates: `exp/mssim/in/s3204_acCmp_branch_rerun_20260501_r{1..5}`
- Round 1 call graphs: `trace-analysis/graph_branch/S_32048416`, `trace-analysis/graph_branch/S_14677443`
- Rounds 2-5 call graphs: original `trace-analysis/golden/S_32048416`, `trace-analysis/golden/S_14677443`
- Round 1: `s3204_acCmp_branch_rerun_20260501_r1`
  - Status: completed successfully.
  - Sum goodput, target policy: 6118.498
  - Original target-policy sum: 9972.833 (`-3854.335`)
  - Original competing-policy match sum: 8811.367 (`-2692.869`)
  - Decision: the branch-graph workload is not comparable to the original plot; use original call graph paths for remaining rounds to evaluate policy/code changes.
- Round 2: `s3204_acCmp_branch_rerun_20260501_r2`
  - Status: completed successfully.
  - Sum goodput, target policy: 11893.965
  - Original target-policy sum: 9972.833 (`+1921.132`)
  - Original competing-policy match sum: 8811.367 (`+3082.598`)
  - Decision: stop after round 2 because it beats both the original target-policy run and the competing-policy match.

### 4. `hotel/test1`

- Baseline source: `exp/hotel/in/test1` and `exp/hotel/plots/test1`
- Duplicates: `exp/hotel/in/test1_branch_rerun_20260501_r{1..5}`
- Round 1: `test1_branch_rerun_20260501_r1`
  - Status: completed successfully.
  - Output: `exp/hotel/out/test1_branch_rerun_20260501_r1`
  - Plots: `exp/hotel/plots/test1_branch_rerun_20260501_r1`
  - Sum goodput, target policy: 7223.800
  - Original target-policy sum: 10542.733 (`-3318.933`)
  - Original competing-policy match sum: 9301.972 (`-2078.172`)
  - Decision: continue; round 1 is below both the original target-policy run and the competing-policy match.
- Round 2: `test1_branch_rerun_20260501_r2`
  - Status: completed successfully.
  - Output: `exp/hotel/out/test1_branch_rerun_20260501_r2`
  - Plots: `exp/hotel/plots/test1_branch_rerun_20260501_r2`
  - Sum goodput, target policy: 7536.359
  - Original target-policy sum: 10542.733 (`-3006.374`)
  - Original competing-policy match sum: 9301.972 (`-1765.613`)
  - Decision: continue; round 2 improved over round 1 but remains below both baselines.
- Round 3: `test1_branch_rerun_20260501_r3`
  - Status: completed successfully.
  - Output: `exp/hotel/out/test1_branch_rerun_20260501_r3`
  - Plots: `exp/hotel/plots/test1_branch_rerun_20260501_r3`
  - Sum goodput, target policy: 7749.475
  - Original target-policy sum: 10542.733 (`-2793.258`)
  - Original competing-policy match sum: 9301.972 (`-1552.497`)
  - Decision: continue; round 3 improved again but remains below both baselines.
- Round 4: `test1_branch_rerun_20260501_r4`
  - Status: completed successfully.
  - Output: `exp/hotel/out/test1_branch_rerun_20260501_r4`
  - Plots: `exp/hotel/plots/test1_branch_rerun_20260501_r4`
  - Sum goodput, target policy: 7437.136
  - Original target-policy sum: 10542.733 (`-3105.597`)
  - Original competing-policy match sum: 9301.972 (`-1864.836`)
  - Decision: continue to the fifth and final allowed round; `fanout_min_samples=1` regressed.
- Round 5: `test1_branch_rerun_20260501_r5`
  - Status: completed successfully.
  - Output: `exp/hotel/out/test1_branch_rerun_20260501_r5`
  - Plots: `exp/hotel/plots/test1_branch_rerun_20260501_r5`
  - Sum goodput, target policy: 7395.158
  - Original target-policy sum: 10542.733 (`-3147.575`)
  - Original competing-policy match sum: 9301.972 (`-1906.814`)
  - Decision: stop at the five-round cap. Best is round 3: 7749.475, which is still 1552.497 below the competing-policy match and 2793.258 below the original target-policy run.

## Final Summary

| Experiment | Rounds run | Best duplicate | Best target-policy sum | Original target-policy sum | Original match sum | Outcome |
|---|---:|---|---:|---:|---:|---|
| `cg8651_acCmp` | 1 | `cg8651_acCmp_branch_rerun_20260501_r1` | 9226.534 | 9065.434 | 7026.268 | Beat original and match. |
| `s1467_acCmp` | 5 | `s1467_acCmp_branch_rerun_20260501_r2` | 4403.966 | 4414.034 | 4139.933 | Beat match, missed original by 10.068. |
| `s3204_acCmp` | 2 | `s3204_acCmp_branch_rerun_20260501_r2` | 11893.965 | 9972.833 | 8811.367 | Beat original and match. |
| `hotel/test1` | 5 | `test1_branch_rerun_20260501_r3` | 7749.475 | 10542.733 | 9301.972 | Missed both; stopped at five-round cap. |

## Hotel New Estimator Tuning

Goal: keep `sched_pred,abort_slack,ac_pred,est_mean_var` and the new latency estimator, but reduce aggressive hotel early returns.

| Iteration | Experiment | `tau_er` | `aimd_er_threshold` | `fanout_min_samples` | `legacy_fraction` | `root_only` | Status |
|---:|---|---:|---:|---:|---:|---|---|
| 1 | `test1_new_est_tune_20260501_i1` | 4 | 0.6 | 8 | 1.0 | true | completed: goodput 7202.995; delta original -3339.738; delta match -2098.977; frontend ER 6064.712 vs original 2886.140 |
| 2 | `test1_new_est_tune_20260501_i2` | 6 | 0.6 | 8 | 1.0 | true | completed: goodput 7161.620; delta original -3381.113; delta match -2140.352; frontend ER 6080.331 vs original 2886.140 |
| 3 | `test1_new_est_tune_20260501_i3` | 4 | 0.8 | 8 | 1.0 | true | completed: goodput 6357.426; delta original -4185.307; delta match -2944.546; frontend ER 6805.230 vs original 2886.140 |

Result: none of the three conservative tuning attempts improved hotel. Best of this set is iteration 1 at 7202.995, which is below the previous best rerun (`test1_branch_rerun_20260501_r3`, 7749.475). Raising `tau_er` and `aimd_er_threshold` did not reduce frontend early returns; frontend ER stayed roughly 2.1x-2.4x the original target-policy run.

## Hotel Abort SLO Check

Goal: keep `sched_pred,ac_pred,est_mean_var` but replace `abort_slack` with `abort_slo`, so requests are aborted only after exceeding their end-to-end SLO.

| Experiment | Policy | Status |
|---|---|---|
| `test1_abort_slo_new_est_20260501` | `sched_pred,abort_slo,ac_pred,est_mean_var` | interrupted before workload; used `{}` params, left partial config-only output |
| `test1_abort_slo_r3params_20260501` | `sched_pred,abort_slo,ac_pred,est_mean_var` with `test1_branch_rerun_20260501_r3` pred params | completed: goodput 7463.359; delta original -3079.374; delta match -1838.613; delta previous best abort_slack -286.116 |

Abort reason comparison:

| Experiment | Top abort reasons |
|---|---|
| `test1_abort_slo_r3params_20260501` | `PredAdmissionRej` 472015.0; `E2EDeadline` 231113.0; `BeforeChildFeasibility` 1888.0 |
| `test1_branch_rerun_20260501_r3` | `PredAdmissionRej` 447666.5; `LocalDeadlineExceeded` 218590.0; `BeforeChildFeasibility` 1734.5 |

Result: switching from `abort_slack` to `abort_slo` did not recover hotel goodput. The abort-SLO run is slightly worse than the prior best abort-slack rerun, and predictive admission rejection remains the dominant recorded early-return reason.

## Hotel Main-Branch Reproduction

Goal: test whether `origin/main` reproduces the previous hotel target-policy goodput without overwriting prior results.

- Worktree: `/mnt/data/shli/masa2-main`
- Commit: `3a3591afd1ae4f8ddf72a58c44455366fe0c9902` (`origin/main`)
- Duplicated input: `exp/hotel/in/test1_main_repro_20260501`
- Output: `exp/hotel/out/test1_main_repro_20260501`
- Plots: `exp/hotel/plots/test1_main_repro_20260501`
- Policy: `sched_pred,abort_slack,ac_pred,est_mean_var`
- Params: inherited from the restored `exp/hotel/in/test1/policy_param.json`
- Status: completed successfully.

| Run | Sum goodput |
|---|---:|
| Original `exp/hotel/plots/test1` target-policy plot | 10542.733 |
| Main-branch reproduction | 10510.419 |
| Current-branch best rerun (`test1_branch_rerun_20260501_r3`) | 7749.475 |

Result: `origin/main` reproduced the previous goodput within 32.314 summed-goodput points (`-0.31%`). It is 2760.944 above the best current-branch hotel rerun, so the hotel regression appears tied to changes after the main-branch commit rather than experiment noise or missing policy params.

## Hotel Closed-Loop Investigation

Goal: understand why the current branch enters a worse hotel equilibrium without sweeping every RPS.

Short duplicated experiments, all using `sched_pred,abort_slack,ac_pred,est_mean_var`, old restored params, and only RPS `[1500, 2000, 3000]`:

| Experiment | Diagnostic change | 1500 total | 2000 total | 3000 total | 3000 Reservation ER/s |
|---|---|---:|---:|---:|---:|
| `test1_loop_oldparams_short_fanout_on_20260501` | branch default | 1067.3 | 801.3 | 745.8 | 1018.7 |
| `test1_loop_oldparams_short_fanout_off_20260501` | `MASA_FANOUT_AWARE=0` | 1257.0 | 802.1 | 776.7 | 996.8 |
| `test1_loop_oldparams_short_full_deadline_20260501` | use full estimate for child deadline again | 990.7 | 803.4 | 773.3 | 996.2 |
| `test1_loop_oldparams_short_hyper_deadline_prio_20260501` | derive Hyper spawn priority from remaining deadline, as `origin/main` did | 1326.2 | 1459.6 | 1626.5 | 373.9 |

Findings:

- Disabling fanout improves the 1500-RPS point but does not recover 2000/3000.
- Restoring the old full-estimate child deadline does not recover 2000/3000.
- Reverting only Hyper's initial H2 stream priority calculation nearly restores the main-branch overload shape in the short run. At 3000 RPS, total goodput rises from 745.8 to 1626.5 and Reservation ER drops from 1018.7/s to 373.9/s.
- The key code change is `libs/hyper/src/proto/h2/server.rs`: current branch uses serialized `ctx.prio_hint()` for spawn priority; `origin/main` derived `PriorityHint::new(ctx.deadline() - time_now())`.
- This appears to be a units/meaning mismatch. `ctx.prio_hint()` is serialized as an absolute timestamp-like deadline (`ctx.deadline() - est_remaining`), while `EstimationLayer::before_poll` reprioritizes running tasks with relative remaining time (`ctx.deadline() - time_now()`). Since smaller `PriorityHint` means higher priority, already-polled tasks with relative values around `0..100000` dominate newly spawned H2 tasks whose serialized absolute hints are around the wall-clock timestamp scale. Under overload, new streams/child RPCs are delayed before their first policy poll, which increases observed child wallclock, triggers more admission rejection/local deadline returns, and keeps the system in a low-goodput equilibrium.

Current source was restored after the diagnostic patches; only duplicated experiment inputs/outputs and this markdown log were added.

## Hotel Remaining Gap After Hyper Revert

Goal: explain the residual gap after reverting Hyper H2 stream spawn priority to deadline-relative priority.

Additional duplicated runs:

- Main short apples-to-apples baseline: `/mnt/data/shli/masa2-main/exp/hotel/in/test1_main_short_3rps_20260501`
- Current branch with committed Hyper revert and fanout disabled: `exp/hotel/in/test1_loop_oldparams_short_hyper_revert_fanout_off_20260501`

Direct short-run comparison:

| RPS | Branch Hyper revert total | Main short total | Delta |
|---:|---:|---:|---:|
| 1500 | 1326.2 | 1392.8 | -66.6 (`-4.8%`) |
| 2000 | 1459.6 | 1528.6 | -69.1 (`-4.5%`) |
| 3000 | 1626.5 | 1692.3 | -65.8 (`-3.9%`) |

At 3000 RPS, the residual gap is mostly extra early return:

| API | Branch OK/s | Main OK/s | Branch ER/s | Main ER/s | Main ER mix |
|---|---:|---:|---:|---:|---|
| Search | 500.9 | 547.5 | 1001.7 | 965.1 | mostly `LocalDeadlineExceeded` |
| Reservation | 1131.5 | 1154.1 | 374.3 | 343.5 | mostly `PredAdmissionRej` |

Frontend estimator differences in the default branch Hyper-revert run:

| Edge | Branch estimate | Main short estimate | Notes |
|---|---:|---:|---|
| `HandleSearch=>search.HandleNearby` remaining work | 142.8 ms avg | 128.4 ms avg | Branch is more pessimistic here. |
| `HandleReservation=>reservation.MakeReservation` child wallclock | 26.1 ms avg | 20.2 ms avg | Branch observes slower reservation child RPCs. |
| `HandleReservation=>user.CheckUser` child wallclock | 1.30 ms avg | 1.14 ms avg | Small branch slowdown. |

Fanout-off with Hyper revert:

| RPS | Fanout-off total | Main short total | Delta |
|---:|---:|---:|---:|
| 1500 | 1338.3 | 1392.8 | -54.5 |
| 2000 | 1427.8 | 1528.6 | -100.8 |
| 3000 | 1619.7 | 1692.3 | -72.6 |

Result: fanout-off normalizes the `HandleSearch=>search.HandleNearby` remaining-work estimate (`126.8 ms` avg vs main `128.4 ms`), but it does not close the goodput gap. The remaining loss appears to come from small added latency/ER sensitivity in the updated branch, not a single fanout-key issue. CPU summaries are very close, but the branch has slightly higher frontend/user CPU and slightly slower successful Search/Reservation latency; because Search runs near its 200 ms SLO and Reservation admission is sensitive to observed child wallclock, a few milliseconds are enough to move 4-5% goodput.

Clean repeat with committed Hyper revert and fanout enabled: `test1_loop_oldparams_short_hyper_revert_repeat_20260501`.

| RPS | First branch total | Repeat branch total | Main short total | Repeat delta vs main |
|---:|---:|---:|---:|---:|
| 1500 | 1326.2 | 1375.5 | 1392.8 | -17.3 (`-1.2%`) |
| 2000 | 1459.6 | 1441.0 | 1528.6 | -87.6 (`-5.7%`) |
| 3000 | 1626.5 | 1625.2 | 1692.3 | -67.1 (`-4.0%`) |

Repeat error breakdown:

| RPS | API | Branch repeat OK/s | Main OK/s | Branch repeat ER/s | Main ER/s |
|---:|---|---:|---:|---:|---:|
| 1500 | Search | 645.5 | 654.3 | 113.1 | 93.7 |
| 1500 | Reservation | 735.9 | 743.2 | 15.2 | 9.3 |
| 2000 | Search | 561.5 | 634.8 | 434.1 | 364.4 |
| 2000 | Reservation | 888.4 | 902.5 | 111.8 | 100.1 |
| 3000 | Search | 501.1 | 547.5 | 992.3 | 965.1 |
| 3000 | Reservation | 1136.0 | 1154.1 | 364.1 | 343.5 |

At 2000 RPS, the extra branch Search ER is mostly `LocalDeadlineExceeded` (`317.1/s` vs main `260.5/s`) plus some `PredAdmissionRej` (`112.8/s` vs `97.2/s`). At 3000 RPS, the branch still loses Search goodput (`501.1/s` vs `547.5/s`) and has slightly more Reservation `PredAdmissionRej` (`357.3/s` vs `336.6/s`).

Successful request latency in the repeat is only slightly slower than main:

| RPS | API | Branch repeat p50/p90/p99 | Main p50/p90/p99 |
|---:|---|---|---|
| 1500 | Search | 138.6 / 163.2 / 192.1 ms | 137.7 / 159.9 / 190.4 ms |
| 1500 | Reservation | 29.1 / 53.2 / 68.8 ms | 28.7 / 52.6 / 66.8 ms |
| 2000 | Search | 144.7 / 172.1 / 195.4 ms | 143.5 / 170.1 / 195.0 ms |
| 2000 | Reservation | 27.8 / 49.4 / 64.0 ms | 26.3 / 48.1 / 62.8 ms |
| 3000 | Search | 146.0 / 177.1 / 197.1 ms | 144.2 / 176.8 / 196.9 ms |
| 3000 | Reservation | 19.6 / 42.6 / 66.9 ms | 18.1 / 42.8 / 69.7 ms |

Temporary timing instrumentation showed direct estimator overhead is not the explanation: on frontend, `begin_child`, `est_after_child_wallclock_for_group`, `record_child_complete`, and `recover_path_groups` averaged `0us`; `flush` averaged `2us`. The timed run itself remained in the same goodput range. Since fanout-off also failed to recover the gap, keep fanout tracking enabled and focus next on the closed-loop policy effect: why the updated estimator causes slightly higher Search deadline misses and Reservation admission rejection after the Hyper priority scale fix.

### Overload-Only Parameter Tuning After Hyper Revert

Goal: check whether the remaining hotel gap can be closed by parameter tuning, using duplicated 2000/3000-RPS runs only.

| Experiment | Change | 2000 total | 3000 total | 2000 delta vs main | 3000 delta vs main |
|---|---|---:|---:|---:|---:|
| `test1_loop_oldparams_short_hyper_revert_repeat_20260501` | baseline branch params | 1441.0 | 1625.2 | -87.7 | -67.1 |
| `test1_tune_overload_less_ac_20260501` | less aggressive admission: threshold `0.40`, beta `0.90`, alpha `0.10`, tau `1.0` | 1438.2 | 1673.4 | -90.4 | -19.0 |
| `test1_tune_overload_more_ac_20260501` | more protective admission: threshold `0.20`, beta `0.80` | 1330.7 | 1467.9 | -197.9 | -224.4 |
| `test1_tune_overload_k1_20260501` | variance-aware soft priority: `estimator_k=1.0` | 1384.8 | 1584.0 | -143.8 | -108.3 |
| `test1_tune_overload_k2_lessac_20260501` | `estimator_k=2.0` plus less aggressive admission | 829.1 | 857.8 | -699.5 | -834.5 |

API-level outcome:

| Experiment | 2000 Search | 2000 Reservation | 3000 Search | 3000 Reservation |
|---|---:|---:|---:|---:|
| Branch baseline | 558.1 | 882.9 | 497.5 | 1127.8 |
| Main short | 631.2 | 897.4 | 544.5 | 1147.8 |
| Less aggressive admission | 518.3 | 920.0 | 481.0 | 1192.4 |
| More protective admission | 539.8 | 791.0 | 460.8 | 1007.2 |
| `estimator_k=1.0` | 537.2 | 847.6 | 473.6 | 1110.4 |
| `estimator_k=2.0` plus less admission | 687.4 | 141.7 | 705.0 | 152.8 |

Interpretation:

- Less aggressive admission can nearly close the 3000-RPS total gap, but it does so by admitting more Reservation work and making Search worse. At 3000 RPS Search drops from `497.5/s` to `481.0/s`, while Reservation rises from `1127.8/s` to `1192.4/s`.
- More protective admission reduces Search `LocalDeadlineExceeded` at 2000 RPS (`317.1/s` baseline to `239.9/s`) but replaces it with more `PredAdmissionRej` and loses total goodput.
- `estimator_k=1.0` worsens both APIs. `estimator_k=2.0` with looser admission strongly favors Search but starves Reservation through `BeforeChildFeasibility` and `PredAdmissionRej`.

Conclusion: the gap is not cleanly fixed by global parameter tuning. The remaining behavior is a policy allocation problem: global admission/scheduling knobs can trade goodput between Search and Reservation, but they do not recover both APIs at both overloaded RPS points. The next code-level direction should be algorithmic, likely API/SLO-aware feedback or admission rather than a single global AIMD state.

### API-Aware Predictive Admission Prototype

Goal: test whether splitting predictive admission feedback by root API can address the Search/Reservation allocation problem.

Code prototype: `libs/masa-policy/src/layer/admission/predictive.rs`.

- First version: per-root AIMD state only.
- Second version: hybrid global + per-root AIMD. Each admitted root outcome updates both the global controller and that root's controller; admission uses the stricter rejection probability. This keeps a shared capacity cap while allowing a root-specific controller to be more protective when one API has worse ER behavior.

Validation:

- `cargo fmt --check`
- `cargo test -p masa-policy --features "sched_pred,abort_slack,ac_pred,est_mean_var" predictive::tests`
- `cargo check -p masa-policy --features "sched_pred,abort_slack,ac_pred,est_mean_var"`

Overload-only hotel runs:

| Experiment | Algorithm | 2000 Search | 2000 Reservation | 2000 total | 3000 Search | 3000 Reservation | 3000 total |
|---|---|---:|---:|---:|---:|---:|---:|
| `test1_loop_oldparams_short_hyper_revert_repeat_20260501` | global-only baseline | 558.1 | 882.9 | 1441.0 | 497.5 | 1127.8 | 1625.2 |
| `test1_main_short_3rps_20260501` | main short baseline | 631.2 | 897.4 | 1528.6 | 544.5 | 1147.8 | 1692.3 |
| `test1_per_root_ac_overload_20260501` | per-root only | 534.5 | 996.3 | 1530.8 | 482.0 | 1496.3 | 1978.3 |
| `test1_hybrid_root_global_ac_overload_20260501` | global cap + per-root | 538.4 | 877.5 | 1415.9 | 560.9 | 1201.9 | 1762.8 |

Error breakdown highlights:

- Per-root-only admits Reservation almost completely (`0.0/s` Reservation ER at 3000), which drives huge total goodput but hurts Search. This confirms that root-specific feedback alone removes too much shared-capacity protection.
- Hybrid restores a Reservation rejection cap (`283.5/s` Reservation `PredAdmissionRej` at 3000) and improves 3000-RPS goodput above both current branch and main short. Search also improves at 3000 (`560.9/s` vs main `544.5/s`).
- Hybrid is worse at 2000 RPS (`1415.9` total vs branch baseline `1441.0` and main `1528.6`) because Search gets too many predictive rejections (`265.4/s` vs baseline `112.8/s`) even though local deadline misses fall (`202.1/s` vs baseline `317.1/s`).

Interpretation:

The API-aware direction is promising but not done. Per-root-only proves that decoupling API feedback can unlock Reservation throughput, but it over-admits Reservation and starves Search. The hybrid global+per-root cap is safer and wins strongly at 3000 RPS, but it is too conservative for Search at 2000 RPS. Next tuning should focus on the hybrid controller's per-root recovery/aggressiveness, e.g. less severe per-root rejection or a different combination rule than `max(global_reject_prob, root_reject_prob)`.

### Hybrid Admission Combination Tuning

Goal: tune the hybrid global+root admission combination rule. Hard `max(global_reject_prob, root_reject_prob)` was too strict at 2000 RPS. Tested a softened rule:

```text
reject_prob = global_reject_prob
            + root_extra_fraction * max(root_reject_prob - global_reject_prob, 0)
```

| Experiment | `root_extra_fraction` | 2000 Search | 2000 Reservation | 2000 total | 3000 Search | 3000 Reservation | 3000 total |
|---|---:|---:|---:|---:|---:|---:|---:|
| `test1_loop_oldparams_short_hyper_revert_repeat_20260501` | baseline global-only | 558.1 | 882.9 | 1441.0 | 497.5 | 1127.8 | 1625.2 |
| `test1_main_short_3rps_20260501` | main short baseline | 631.2 | 897.4 | 1528.6 | 544.5 | 1147.8 | 1692.3 |
| `test1_hybrid_root_global_ac_overload_20260501` | 1.00 (`max`) | 538.4 | 877.5 | 1415.9 | 560.9 | 1201.9 | 1762.8 |
| `test1_hybrid_soft05_ac_overload_20260501` | 0.50 | 547.7 | 885.5 | 1433.2 | 493.8 | 1179.7 | 1673.5 |
| `test1_hybrid_soft075_ac_overload_20260501` | 0.75 | 558.0 | 903.5 | 1461.5 | 484.0 | 1193.5 | 1677.4 |

Error breakdown highlights:

- `0.50` is too soft: it improves 2000 RPS slightly over hard max but loses most of the 3000-RPS win.
- `0.75` is the best balanced point in this small pass: it beats the current branch baseline at both overloaded points (`+20.5` at 2000, `+52.2` at 3000) and improves Reservation above main at both points, but it still underperforms main total because Search remains weak.
- Hard max is best if only optimizing 3000 RPS, but it regresses 2000 RPS below the current branch baseline.

Current code is left at `root_extra_fraction = 0.75` because it avoids the 2000-RPS regression while improving the branch at 3000. This is a better local point, not a complete fix: Search still needs a better recovery/admission shape.
