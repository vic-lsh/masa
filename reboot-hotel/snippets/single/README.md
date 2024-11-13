# README

Folder `single` runs one of `search`/`reservation` in one of `fifo`/`e2e`/`e2e_er`/`local`/`local_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.
* `local`: Priority queue using local deadlines/latest_exec_at.
* `local_er`: `local` plus early return.

## Search

### Ongoing

![search fifo local_er](cmp/fig_goodput_rps_cmp_fifo_search_local_er_search_0.png)
![search e2e_er local_er](cmp/fig_goodput_rps_cmp_e2e_er_search_local_er_search_0.png)
![search local local_er](cmp/fig_goodput_rps_cmp_local_search_local_er_search_0.png)

### Done

![fifo e2e](cmp/fig_goodput_rps_cmp_fifo_search_e2e_search_0.png)
![fifo e2e_er](cmp/fig_goodput_rps_cmp_fifo_search_e2e_er_search_0.png)
![e2e e2e_er](cmp/fig_goodput_rps_cmp_e2e_search_e2e_er_search_0.png)

<!-- ![fifo](fifo_search/fig_goodput_rps_0.png)
![e2e](e2e_search/fig_goodput_rps_0.png)
![e2e_er](e2e_er_search/fig_goodput_rps_0.png) -->

## Reservation

### Ongoing

![reservation fifo local_er](cmp/fig_goodput_rps_cmp_fifo_reservation_local_er_reservation_0.png)
![reservation e2e_er local_er](cmp/fig_goodput_rps_cmp_e2e_er_reservation_local_er_reservation_0.png)
![reservation local local_er](cmp/fig_goodput_rps_cmp_local_reservation_local_er_reservation_0.png)

### Done

![fifo e2e](cmp/fig_goodput_rps_cmp_fifo_reservation_e2e_reservation_0.png)
![fifo e2e_er](cmp/fig_goodput_rps_cmp_fifo_reservation_e2e_er_reservation_0.png)
![e2e e2e_er](cmp/fig_goodput_rps_cmp_e2e_reservation_e2e_er_reservation_0.png)

<!-- ![fifo](fifo_reservation/fig_goodput_rps_0.png)
![e2e](e2e_reservation/fig_goodput_rps_0.png)
![e2e_er](e2e_er_reservation/fig_goodput_rps_0.png) -->
