#!/usr/bin/env bash
# Script to run RepCXL's YCSB client benchmark. Might need super user privileges 
# to access memory nodes
set -e

REPCXL_ID=$1
WORKLOAD=$2
CONFIG=$3

if [ -z "$REPCXL_ID" ]; then
    echo "Usage: $0 <repcxl_id> <workload> <config>"
    exit 1
fi

if [ -z "$WORKLOAD" ]; then
    WORKLOAD=workloada_64
fi

export RUST_LOG=info 

target/release/ycsb_client \
    ycsb/traces/${WORKLOAD}_load.dat \
    ycsb/traces/${WORKLOAD}_run.dat \
    --config ${CONFIG} \
    --id ${REPCXL_ID}
