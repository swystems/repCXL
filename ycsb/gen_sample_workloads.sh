#!/usr/bin/env bash

YCSB_DIR=$1
REPO_DIR=$2

if [ -z "$YCSB_DIR" ]; then
    YCSB_DIR=~/ycsb/
fi

if [ ! -d "$YCSB_DIR" ]; then
    echo "YCSB directory $YCSB_DIR does not exist."
    exit 1
fi

if [ ! -d "$REPO_DIR" ]; then
    REPO_DIR=$(pwd)
fi

# generate workload files
cd $YCSB_DIR # required for retarded Maven build system
# workload A
bin/ycsb load basic -P $REPO_DIR/bench/ycsb-workloads/workloada_64 > $REPO_DIR/bench/ycsb-traces/workloada_64_load.log
bin/ycsb run basic -P $REPO_DIR/bench/ycsb-workloads/workloada_64 > $REPO_DIR/bench/ycsb-traces/workloada_64_run.log
# workload B
bin/ycsb load basic -P $REPO_DIR/bench/ycsb-workloads/workloadb_64 > $REPO_DIR/bench/ycsb-traces/workloadb_64_load.log
bin/ycsb run basic -P $REPO_DIR/bench/ycsb-workloads/workloadb_64 > $REPO_DIR/bench/ycsb-traces/workloadb_64_run.log
# workload A 1 record
bin/ycsb load basic -P $REPO_DIR/bench/ycsb-workloads/workloada_64_single > $REPO_DIR/bench/ycsb-traces/workloada_64_single_load.log
bin/ycsb run basic -P $REPO_DIR/bench/ycsb-workloads/workloada_64_single > $REPO_DIR/bench/ycsb-traces/workloada_64_single_run.log

cd -