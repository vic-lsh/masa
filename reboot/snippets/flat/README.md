# README

Assert `Masa <= Fifo` when:

* All processing latency is static as 16ms.
* Use constant load generation.
* Use one hop.
* Let `Masa` enable `deadline`.

# Tiny Observations

* Expected results. No queueing, no difference.

# Tiny Sparks

* Make first hop static and second hop exponential.
* Make first hop exponential and second hop static.
