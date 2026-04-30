#!/usr/bin/env bash
# Script to run RepCXL's YCSB client benchmark. Might need super user privileges 
# to access memory nodes
# Assumes YCSB workloads formatted as <workload>_{load,run}<client_id>. Hence for 
# 32 clients it looks for <workload>_{load,run}<0..31>
set -e

if [ $# -ne 3 ]; then
    echo "Usage: $0 <number_of_clients> <workload> <config>"
    exit 1
fi

NCLIENTS=$1
WORKLOAD=$2
CONFIG=$3


if [ $NCLIENTS -lt 2 ]; then
    echo "Number of clients must be at least 2"
    exit 1
fi

export RUST_LOG=info 

run_ycsb_client() {
    local node="$1"

    # cheeky config: isolated on the machine 20-120 so 20+id should do for 1-100 clients
    local core=$((20 + node))

    taskset -c "$core" target/release/ycsb_client \
        "ycsb/traces/${WORKLOAD}_load${node}.dat" \
        "ycsb/traces/${WORKLOAD}_run${node}.dat" \
        --config "${CONFIG}" \
        --id "$node" \
        > "bench_out${node}.dat" 2>&1
}

# start coordinator and wait for it init state and create objects
run_ycsb_client 0 &
sleep 2 # might need to be adjusted for large workloads

for node in $(seq 1 $((NCLIENTS-1))); do
    run_ycsb_client "$node" &
done

# Wait for all background processes to complete
wait
