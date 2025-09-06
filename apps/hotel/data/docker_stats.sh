#!/usr/bin/env bash
OUT="container_stats.csv"
echo "ts_ms,container,cpu_perc,mem_usage,net_io,block_io,pids" > "$OUT"

while :; do
  TS=$(date +%s%3N)
  docker stats --no-stream \
    --format "{{.Name}},{{.CPUPerc}},{{.MemUsage}},{{.NetIO}},{{.BlockIO}},{{.PIDs}}" \
    | awk -v ts="$TS" -F',' '{print ts","$0}' >> "$OUT"
  sleep 1
done