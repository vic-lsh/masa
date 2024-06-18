# python src/sim/main.py \
#   --rps 400 \
#   --secs 30 \
#   --output1 snippets/hotnets/fcfs.csv \
#   --output2 snippets/hotnets/masa.csv

python src/sim/test.py \
  --mode fcfs \
  --rps 350 \
  --secs 60 \
  --output snippets/hotnets/fcfs.csv

python src/sim/test.py \
  --mode masa \
  --rps 350 \
  --secs 60 \
  --output snippets/hotnets/masa.csv
