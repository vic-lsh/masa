cargo run --release --bin charleston_lactose -- \
  --depth 6 \
  --exec-mus 3000 3000 3000 3000 3000 3000 \
  --replicas 6 \
  --mode fcfs \
  --rps 1600 \
  --secs 60 \
  --concurrency 256 \
  --output snippets/hotnets/fcfs.csv

cargo run --release --bin charleston_lactose -- \
  --depth 6 \
  --exec-mus 3000 3000 3000 3000 3000 3000 \
  --replicas 6 \
  --mode masa \
  --rps 1600 \
  --secs 60 \
  --concurrency 256 \
  --output snippets/hotnets/masa.csv
