# README

Folder `single` runs one of `search`/`reservation` in one of `fifo`/`e2e`/`e2e_er`/`local`/`local_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.
* `local`: Priority queue using local deadlines/latest_exec_at.
* `local_er`: `local` plus early return.

## Search

### Ongoing

<!-- ![fifo e2e e2e_er local local_er](cmp/fig_goodput_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![mean fifo e2e e2e_er local local_er](cmp/fig_mean_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p50 fifo e2e e2e_er local local_er](cmp/fig_p50_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p90 fifo e2e e2e_er local local_er](cmp/fig_p90_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p95 fifo e2e e2e_er local local_er](cmp/fig_p95_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png)
![p99 fifo e2e e2e_er local local_er](cmp/fig_p99_total_rps_cmp_fifo_e2e_e2e_er_local_local_er_0.png) -->
