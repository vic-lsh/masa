# README

Folder `single` runs one of `search`/`reservation` in one of `fifo`/`e2e`/`e2e_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.

## Search

![fifo e2e](cmp/fig_goodput_rps_cmp_fifo_search_e2e_search_0.png)
![fifo e2e_er](cmp/fig_goodput_rps_cmp_fifo_search_e2e_er_search_0.png)
![e2e e2e_er](cmp/fig_goodput_rps_cmp_e2e_search_e2e_er_search_0.png)
![fifo](fifo_search/fig_goodput_rps_0.png)
![e2e](e2e_search/fig_goodput_rps_0.png)
![e2e_er](e2e_er_search/fig_goodput_rps_0.png)

## Reservation

![fifo e2e](cmp/fig_goodput_rps_cmp_fifo_reservation_e2e_reservation_0.png)
![fifo e2e_er](cmp/fig_goodput_rps_cmp_fifo_reservation_e2e_er_reservation_0.png)
![e2e e2e_er](cmp/fig_goodput_rps_cmp_e2e_reservation_e2e_er_reservation_0.png)
![fifo](fifo_reservation/fig_goodput_rps_0.png)
![e2e](e2e_reservation/fig_goodput_rps_0.png)
![e2e_er](e2e_er_reservation/fig_goodput_rps_0.png)
