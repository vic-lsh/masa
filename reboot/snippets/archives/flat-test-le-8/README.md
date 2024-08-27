# README

Assert `Masa <= Fifo` when:

1. All processing latency is static as 16ms.
2. Let `Masa` enable `deadline`.
3. Let `Fifo` enable `ConcurrentFifoQueue`.

# Tiny Observations

1. P99 difference is close to 16ms.
2. Let `Masa` disable `deadline`.

# Tiny Sparks

1. Vary processing latency.
2. Make first hop static and second hop exponential.
3. Make first hop exponential and second hop static.
