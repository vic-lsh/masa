# README

Folder `search_reservation` runs `search:reservation=1:1` in one of `fifo`/`e2e`/`e2e_er`/`local`/`local_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.
* `local`: Priority queue using local deadlines/latest_exec_at.
* `local_er`: `local` plus early return.

[TODO:LD]: We should use different percentiles for local deadlines and latest_exec_at.

## Ongoing

![fifo e2e e2e_er local local_er](cmp/fig_goodput_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![mean fifo e2e e2e_er local local_er](cmp/fig_mean_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p90 fifo e2e e2e_er local local_er](cmp/fig_p90_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p95 fifo e2e e2e_er local local_er](cmp/fig_p95_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p99 fifo e2e e2e_er local local_er](cmp/fig_p99_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)

## Done

![fifo e2e local](cmp/fig_goodput_rps_cmp_fifo_e2e_local_0.png)
![fifo e2e_er local_er](cmp/fig_goodput_rps_cmp_fifo_e2e_er_local_er_0.png)
![local_p50 local_p75 local_p90 local_p95](cmp/fig_goodput_rps_cmp_local_p50_local_p75_local_p90_local_p95_0.png)
![local_er_p50 local_er_p75 local_er_p90 local_er_p95](cmp/fig_goodput_rps_cmp_local_er_p50_local_er_p75_local_er_p90_local_er_p95_0.png)
