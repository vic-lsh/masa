# README

[TODO].

Folder `single` runs one of `search`/`reservation` in one of `fifo`/`e2e`/`e2e_er`.

* `fifo`: Single FIFO queue for both normal and infra tasks.
* `e2e`: Priority queue using e2e deadlines.
* `e2e_er`: `e2e` plus early return.

![fifo search](fifo_search/fig_goodput_rps_bar_0.png)
![fifo reservation](fifo_reservation/fig_goodput_rps_bar_0.png)
![e2e search](e2e_search/fig_goodput_rps_bar_0.png)
![e2e reservation](e2e_reservation/fig_goodput_rps_bar_0.png)
![e2e_er search](e2e_er_search/fig_goodput_rps_bar_0.png)
![e2e_er reservation](e2e_er_reservation/fig_goodput_rps_bar_0.png)
