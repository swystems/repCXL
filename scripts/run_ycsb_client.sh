#!/usr/bin/env bash
# Script to run RepCXL's YCSB client benchmark. Might need super user privileges 
# to access memory nodes
set -e

VM_ID=$1
WORKLOAD=$2
CONFIG=$3

if [ -z "$VM_ID" ]; then
    echo "Usage: $0 <vm_id>"
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
    --id ${VM_ID}
