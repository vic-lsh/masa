# README

Failed to assert `Masa <= Fifo` when:

1. All processing latency is static as 16ms.
2. Let `Masa` disable `deadline`.

# Tiny Observations

1. When `deadline` is disabled in `Masa`, the tail becomes much worse. This is expected since the priority queue is not able to pop futures of earlier requests.

# Tiny Sparks

1. Vary processing latency.
2. Make first hop static and second hop exponential.
3. Make first hop exponential and second hop static.
