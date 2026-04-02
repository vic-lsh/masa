# VOLT — Predictive AC v2 evaluation

## Key questions
- Does the v2 design (accumulated compute cost, root_method keying, tightened feasibility, early feasibility check) improve goodput over anchor_20 (v1)?
- Which of the four v2 changes contributes most to any observed improvement or regression?
- Does the compute-only cost signal eliminate the positive feedback loop under congestion?

## Pre-v2 baseline: anchor_20 (socialnet)

Policy: `sched_pred,abort_slo,ac_pred,est_mean_var`
SLO: 50ms, API: ComposePost

| RPS  | Goodput (v1) | Fraction |
|------|-------------|----------|
| 800  | 799.8       | 100.0%   |
| 1200 | 942.3       | 78.5%    |
| 1400 | 966.7       | 69.1%    |
| 1800 | 1106.4      | 61.5%    |
| 2500 | 1209.5      | 48.4%    |

Early returns (frontend → ComposePost):
- 800: 0.2/s, 1200: 257.5/s, 1400: 433.1/s, 1800: 692.4/s, 2500: 1290.1/s

## Experiment series: volt_1, volt_2, ... (socialnet)
