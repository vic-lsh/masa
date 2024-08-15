# README

Assert `Masa <= Fifo` when:

* Use one hop.
* Use one executor thread.
* Use static processing latency as 16ms.
* Enable `deadline` in `Masa`.

# Tiny Observations

* Again very interesting results. Is it possible to simulate `Fifo` in `Masa`'s priority queue?

# Tiny Sparks

* Make first hop static and second hop exponential.
* Make first hop exponential and second hop static.
