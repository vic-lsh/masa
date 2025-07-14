# Microservices Evaluator

This project evaluates microservices with various synthetic workloads.

It supports:

1. Arbitrary call graph definitions via a config file
2. Config file-defined service time distribution per service
3. Custom load generation logic for this call graph

## Getting started

Minimal example to run this project:

``` bash
$ cd runner
$ cargo run -- --input ./test_config.json
```
