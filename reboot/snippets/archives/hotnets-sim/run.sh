# python src/sim/main.py \
#   --rps 400 \
#   --secs 30 \
#   --output1 snippets/hotnets-sim/fcfs.csv \
#   --output2 snippets/hotnets-sim/masa.csv

python src/sim/test.py \
  --mode fcfs \
  --rps 350 \
  --secs 60 \
  --output snippets/hotnets-sim/fcfs.csv

python src/sim/test.py \
  --mode masa \
  --rps 350 \
  --secs 60 \
  --output snippets/hotnets-sim/masa.csv
