# README

Folder `search_reservation` runs `search:reservation=1:1` in one of `fifo`/`e2e`/`e2e_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.

![fifo e2e](cmp/fig_goodput_rps_cmp_fifo_e2e_0.png)
![fifo e2e_er](cmp/fig_goodput_rps_cmp_fifo_e2e_er_0.png)
![e2e e2e_er](cmp/fig_goodput_rps_cmp_e2e_e2e_er_0.png)
![fifo apis](fifo/fig_goodput_apis_rps_0.png)
![e2e apis](e2e/fig_goodput_apis_rps_0.png)
![e2e_er apis](e2e_er/fig_goodput_apis_rps_0.png)
