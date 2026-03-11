# CORAL — prio_local,est_mean_var,emp_admission,early

## Key questions
- Can empirical admission control (emp_admission) close the goodput gap between prio_local,est_mean_var,early (−124 at 1400 RPS) and prio_local,early under Hotel SLO=50ms?
- Does the empirical P(complete | api, bucket) mechanism correctly suppress over-shedding of Reservation requests while still shedding hopeless requests?
- Can prio_local,est_mean_var,emp_admission,early eventually beat prio_local,early (our previous best) and prio_oldest,early (best-in-class baseline)?

## Experiment series: coral_1, coral_2, ... (hotel)

## Reference: pine_10_lowslo baseline

Existing results from pine_10_lowslo (SLO=50ms, 60s/step, 20s warmup):

| RPS  | prio_local,early | prio_local,emv,early | prio_oldest,early |
|------|-----------------|----------------------|-------------------|
| 100  | 49.6            | 48.5                 | 50.8              |
| 400  | 202.0           | 199.6                | 198.1             |
| 800  | 401.2           | 400.0                | 397.4             |
| 1200 | 584.6           | 559.5                | 561.6             |
| 1400 | **525.3**       | 400.9                | 456.2             |
| 1600 | 429.2           | 399.2                | 358.1             |
| 1800 | 427.2           | 383.7                | 351.0             |
| 2000 | 439.5           | 418.0                | 387.2             |

Notes:
- `prio_local,early` (ple) is currently best at all overloaded points
- `prio_local,est_mean_var,early` (emv) without emp_admission is −124 vs ple at 1400
- emp_admission is designed to replace the est_remaining-based ER threshold with empirical P(complete)
- This should prevent over-shedding of Reservation requests (the design doc's central motivation)

---

## Iteration 1: Baseline validation of emp_admission (coral_1)

**Status:** Pending

### Change
No code change. First experiment to observe emp_admission behavior vs baselines.

### Hypothesis
emp_admission should substantially reduce Reservation early-return at overloaded RPS points vs
emv without emp_admission. Whether it matches or beats prio_local,early is the key question.
At minimum it should outperform prio_local,est_mean_var,early (emv alone).

### Expected outcomes if hypothesis is correct:
1. emv+emp at 1400 RPS: goodput significantly above emv alone (400.9) — ideally approaching ple (525.3)
2. Reservation ER rate at 1400 drops from emv's ~173/s toward ple's ~130/s
3. No regression at 100–800 RPS (where all policies converge to near-full throughput)

### Experiment design
Run coral_1 with simplified RPS sweep [800, 1200, 1400, 1600, 1800] — 5 steps to keep runtime
short (~6 min). Policies: prio_local,early; prio_oldest,early; prio_local,est_mean_var,emp_admission,early.
Omit fifo and emv-without-emp (we have reference data from pine_10_lowslo).
Duration: 60s/step (same as pine_10_lowslo), warmup 20s.
