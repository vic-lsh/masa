# README 0827

* As expected, infra tasks have shorter queueing latency in Masa, since they are prioritized over req tasks.
* As expected, req tasks have longer queueing latency in Masa, since they are queued behind req tasks.
* This data might need more preprocessing since an actual request consists of many req tasks.
* Intuitively, the queueing distribution of all tasks is the sum of the two distributions for infra and req tasks. It is almost impossible to predict the sum of two distributions, if Masa is better or not.
* The p99 only presents an overall trend and does not directly reflect actual requests.
* It is challenging to make strong conclusions in micro-benchmarking. Micro-benchmarking is used for simple but effective sanity checks. System support is built along the journey.
