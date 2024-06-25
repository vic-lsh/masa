cargo run --release --bin charleston_lactose -- \
  --depth 10 \
  --exec-ks 1 1 1 1 1 1 1 1 1 1 \
  --first-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
  --second-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
  --first-slo 50000 \
  --second-slo 500000 \
  --replicas 4 \
  --mode fcfs \
  --rps 1600 \
  --secs 60 \
  --concurrency 256 \
  --output snippets/hotnets/fcfs.csv

cargo run --release --bin charleston_lactose -- \
  --depth 10 \
  --exec-ks 1 1 1 1 1 1 1 1 1 1 \
  --first-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
  --second-exec-mus 2000 2000 2000 2000 2000 2000 2000 2000 2000 2000 \
  --first-slo 50000 \
  --second-slo 500000 \
  --replicas 4 \
  --mode masa \
  --rps 1600 \
  --secs 60 \
  --concurrency 256 \
  --output snippets/hotnets/masa.csv

# --exec-ks 0.125 0.125 0.125 0.125 0.125 0.125 \
# --exec-ks 0.25 0.25 0.25 0.25 0.25 0.25 \
# --exec-ks 0.5 0.5 0.5 0.5 0.5 0.5 \
# --exec-ks 1 1 1 1 1 1 \

# --first-exec-mus 3000 3000 3000 3000 3000 3000 \
# --second-exec-mus 3000 3000 3000 3000 3000 3000 \

# --first-exec-mus 1500 2000 2500 3500 4000 4500 \
# --second-exec-mus 4500 4000 3500 2500 2000 1500 \

# --first-exec-mus 1500 1500 1500 4500 4500 4500 \
# --second-exec-mus 4500 4500 4500 1500 1500 1500 \
