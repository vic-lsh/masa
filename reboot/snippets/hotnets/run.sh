cargo run --release --bin charleston_lactose -- \
  --mode fcfs \
  --rps 1000 \
  --secs 60 \
  --concurrency 32 \
  --output snippets/hotnets/fcfs.csv

cargo run --release --bin charleston_lactose -- \
  --mode masa \
  --rps 1000 \
  --secs 60 \
  --concurrency 32 \
  --output snippets/hotnets/masa.csv

# cargo run --release --bin charleston_rivas -- \
#   --mode fcfs \
#   --rps 450 \
#   --secs 60 \
#   --concurrency 32 \
#   --output snippets/hotnets/fcfs.csv

# cargo run --release --bin charleston_rivas -- \
#   --mode masa \
#   --rps 450 \
#   --secs 60 \
#   --concurrency 32 \
#   --output snippets/hotnets/masa.csv
