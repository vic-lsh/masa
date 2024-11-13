# README

Folder `search_reservation` runs `search:reservation=1:1` in one of `fifo`/`e2e`/`e2e_er`/`local`/`local_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.
* `local`: Priority queue using local deadlines/latest_exec_at.
* `local_er`: `local` plus early return.

[TODO]: We should use different percentiles for local deadlines and latest_exec_at.

## Ongoing

![fifo local_er](cmp/fig_goodput_rps_cmp_fifo_local_er_0.png)
![local local_er](cmp/fig_goodput_rps_cmp_local_local_er_0.png)
![e2e_er local_er](cmp/fig_goodput_rps_cmp_e2e_er_local_er_0.png)

## Done

![fifo e2e](cmp/fig_goodput_rps_cmp_fifo_e2e_0.png)
![fifo e2e_er](cmp/fig_goodput_rps_cmp_fifo_e2e_er_0.png)
![e2e e2e_er](cmp/fig_goodput_rps_cmp_e2e_e2e_er_0.png)
![fifo apis](fifo/fig_goodput_apis_rps_0.png)
![e2e apis](e2e/fig_goodput_apis_rps_0.png)
![e2e_er apis](e2e_er/fig_goodput_apis_rps_0.png)
