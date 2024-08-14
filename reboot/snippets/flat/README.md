# README

Assert `Masa = Fifo` when:

1. All processing latency is static as 2ms.
2. `Masa` uses an increasing `test_id` as `deadline`.

# Tiny Observations

1. gRPC also has a latency around 2ms. I should change processing latency to verify the impacts of gRPC.
