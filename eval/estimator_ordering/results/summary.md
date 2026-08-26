### Outcomes

| Experiment | Policy | RPS | Goodput | Latency p50/p90 ms | Queue p50/p90 ms | Short SLO | Long SLO |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| estimator_ordering_corrupt_0 | eval_oracle_continuation | 220 | 149.140 | 43.566/54.081 | 0.888/2.280 | 100.000 | 36.920 |
| estimator_ordering_corrupt_10 | eval_oracle_continuation | 220 | 152.626 | 41.960/53.763 | 0.872/2.181 | 100.000 | 39.107 |
| estimator_ordering_corrupt_25 | eval_oracle_continuation | 220 | 151.592 | 41.069/54.271 | 0.833/2.119 | 100.000 | 38.253 |
| estimator_ordering_corrupt_50 | eval_oracle_continuation | 220 | 150.463 | 43.522/53.617 | 0.835/1.924 | 100.000 | 37.902 |
| estimator_ordering_hidden | eval_estimator_audit,est_mean_var | 220 | 151.473 | 31.026/54.119 | 0.701/1.615 | 100.000 | 37.591 |
| estimator_ordering_hidden | eval_oracle_continuation | 220 | 152.559 | 41.008/53.868 | 0.855/2.143 | 100.000 | 39.010 |
| estimator_ordering_hidden | sched_pred,est_mean_var | 220 | 146.127 | 44.188/53.989 | 0.623/1.578 | 100.000 | 34.979 |
| estimator_ordering_hidden | sched_slo | 220 | 149.168 | 30.647/54.163 | 0.925/2.093 | 100.000 | 35.107 |
| estimator_ordering_partial | eval_estimator_audit,est_mean_var | 220 | 152.114 | 30.870/54.165 | 0.767/1.702 | 100.000 | 37.684 |
| estimator_ordering_partial | eval_oracle_continuation | 220 | 155.457 | 30.863/53.457 | 0.857/2.288 | 100.000 | 41.383 |
| estimator_ordering_partial | sched_pred,est_mean_var | 220 | 150.271 | 42.875/53.834 | 0.708/1.649 | 100.000 | 37.597 |
| estimator_ordering_partial | sched_slo | 220 | 151.315 | 42.576/53.893 | 0.885/2.136 | 100.000 | 38.087 |
| estimator_ordering_scale_05 | eval_estimator_audit,est_mean_var | 220 | 148.988 | 30.325/54.463 | 1.065/2.341 | 100.000 | 33.438 |
| estimator_ordering_scale_2 | eval_estimator_audit,est_mean_var | 220 | 148.995 | 42.005/54.255 | 0.786/1.836 | 100.000 | 36.249 |
| estimator_ordering_scale_4 | eval_estimator_audit,est_mean_var | 220 | 149.355 | 30.014/54.121 | 0.809/1.757 | 100.000 | 33.080 |
| estimator_ordering_shadow | eval_estimator_audit,est_mean_var | 100 | 64.144 | 44.910/55.049 | 0.898/2.251 | 100.000 | 29.922 |
| estimator_ordering_shadow | eval_estimator_audit,est_mean_var | 280 | 188.923 | 43.328/54.398 | 0.694/1.848 | 100.000 | 37.189 |
| estimator_ordering_visible | eval_estimator_audit,est_hist | 220 | 145.422 | 42.070/54.428 | 1.008/2.481 | 100.000 | 32.913 |
| estimator_ordering_visible | eval_estimator_audit,est_mean_var | 220 | 150.222 | 31.532/54.130 | 0.788/2.078 | 100.000 | 36.513 |
| estimator_ordering_visible | eval_estimator_audit,est_rms | 220 | 148.487 | 40.509/54.273 | 0.978/2.439 | 100.000 | 35.248 |
| estimator_ordering_visible | eval_oracle_continuation | 220 | 151.004 | 30.621/54.119 | 0.862/2.359 | 100.000 | 36.665 |
| estimator_ordering_visible | sched_pred,est_mean_var | 220 | 149.566 | 30.915/54.112 | 0.756/1.848 | 100.000 | 35.631 |
| estimator_ordering_visible | sched_slo | 220 | 146.264 | 44.066/54.058 | 0.897/2.117 | 100.000 | 35.191 |

### Estimator ordering

| Experiment | Policy | RPS | NAE p50/p90 | Policy NAE p50/p90 | Pair agree | Top agree | Ties | Cross flips | Same-key >5% |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| estimator_ordering_corrupt_0 | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_corrupt_10 | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_corrupt_25 | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_corrupt_50 | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_hidden | eval_estimator_audit,est_mean_var | 220 | 0.190/0.417 | 0.190/0.417 | -- | 17.433 | 100.000 | -- | 64.121 |
| estimator_ordering_hidden | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_hidden | sched_pred,est_mean_var | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_hidden | sched_slo | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_partial | eval_estimator_audit,est_mean_var | 220 | 0.069/0.407 | 0.069/0.407 | 86.067 | 33.902 | 60.197 | 13.933 | 51.741 |
| estimator_ordering_partial | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_partial | sched_pred,est_mean_var | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_partial | sched_slo | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_scale_05 | eval_estimator_audit,est_mean_var | 220 | 0.026/0.101 | 0.020/0.317 | 100.000 | 39.317 | 40.395 | 0.000 | 27.147 |
| estimator_ordering_scale_2 | eval_estimator_audit,est_mean_var | 220 | 0.026/0.104 | 0.400/0.611 | 100.000 | 38.227 | 41.191 | 0.000 | 29.417 |
| estimator_ordering_scale_4 | eval_estimator_audit,est_mean_var | 220 | 0.026/0.090 | 0.235/1.679 | 99.965 | 40.042 | 41.514 | 0.000 | 25.969 |
| estimator_ordering_shadow | eval_estimator_audit,est_mean_var | 100 | 0.028/0.102 | 0.028/0.102 | 100.000 | 35.099 | 40.427 | 0.000 | 29.714 |
| estimator_ordering_shadow | eval_estimator_audit,est_mean_var | 280 | 0.027/0.105 | 0.027/0.105 | 97.583 | 37.072 | 38.387 | 0.000 | 29.602 |
| estimator_ordering_visible | eval_estimator_audit,est_hist | 220 | 0.040/0.072 | 0.390/0.590 | -- | 17.464 | 100.000 | -- | 27.723 |
| estimator_ordering_visible | eval_estimator_audit,est_mean_var | 220 | 0.027/0.101 | 0.027/0.101 | 100.000 | 38.410 | 40.763 | 0.000 | 27.643 |
| estimator_ordering_visible | eval_estimator_audit,est_rms | 220 | 0.040/0.151 | 0.390/0.510 | -- | 16.737 | 100.000 | -- | 28.817 |
| estimator_ordering_visible | eval_oracle_continuation | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_visible | sched_pred,est_mean_var | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |
| estimator_ordering_visible | sched_slo | 220 | --/-- | --/-- | -- | -- | -- | -- | -- |

### Shadow feasibility

| Experiment | Policy | RPS | Learned P/R/FPR | Reference P/R/FPR |
| --- | --- | ---: | ---: | ---: |
| estimator_ordering_corrupt_0 | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_corrupt_10 | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_corrupt_25 | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_corrupt_50 | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_hidden | eval_estimator_audit,est_mean_var | 220 | --/0.000/0.000 | 100.000/47.271/0.000 |
| estimator_ordering_hidden | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_hidden | sched_pred,est_mean_var | 220 | --/--/-- | --/--/-- |
| estimator_ordering_hidden | sched_slo | 220 | --/--/-- | --/--/-- |
| estimator_ordering_partial | eval_estimator_audit,est_mean_var | 220 | 91.304/2.065/0.087 | 100.000/44.936/0.000 |
| estimator_ordering_partial | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_partial | sched_pred,est_mean_var | 220 | --/--/-- | --/--/-- |
| estimator_ordering_partial | sched_slo | 220 | --/--/-- | --/--/-- |
| estimator_ordering_scale_05 | eval_estimator_audit,est_mean_var | 220 | 83.673/3.857/0.357 | 99.798/46.378/0.045 |
| estimator_ordering_scale_2 | eval_estimator_audit,est_mean_var | 220 | 88.732/5.921/0.357 | 100.000/43.985/0.000 |
| estimator_ordering_scale_4 | eval_estimator_audit,est_mean_var | 220 | 85.714/5.104/0.401 | 100.000/43.762/0.000 |
| estimator_ordering_shadow | eval_estimator_audit,est_mean_var | 100 | 88.889/11.830/0.831 | 100.000/51.017/0.000 |
| estimator_ordering_shadow | eval_estimator_audit,est_mean_var | 280 | 93.243/5.062/0.176 | 100.000/41.306/0.000 |
| estimator_ordering_visible | eval_estimator_audit,est_hist | 220 | --/0.000/0.000 | 100.000/47.090/0.000 |
| estimator_ordering_visible | eval_estimator_audit,est_mean_var | 220 | 92.308/6.890/0.266 | 100.000/44.115/0.000 |
| estimator_ordering_visible | eval_estimator_audit,est_rms | 220 | --/0.000/0.000 | 100.000/46.779/0.000 |
| estimator_ordering_visible | eval_oracle_continuation | 220 | --/--/-- | --/--/-- |
| estimator_ordering_visible | sched_pred,est_mean_var | 220 | --/--/-- | --/--/-- |
| estimator_ordering_visible | sched_slo | 220 | --/--/-- | --/--/-- |
