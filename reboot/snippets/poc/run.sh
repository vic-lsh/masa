cargo run --release --bin charleston_client_bench -- \
  --rps 100 \
  --secs 10 \
  --concurrency 32 \
  --addr1 http://[::1]:50051 \
  --addr2 http://[::1]:50052 \
  --output1 snippets/poc/client1.csv \
  --output2 snippets/poc/client2.csv

cargo run --release --bin charleston_server -- \
  --num-threads 1
