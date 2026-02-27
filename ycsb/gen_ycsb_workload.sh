#!/usr/bin/env bash

YCSB_DIR=$1
REPO_DIR=$(pwd)

if [ -z "$YCSB_DIR" ]; then
    YCSB_DIR=~/ycsb/
fi

# generate workload files
cd $YCSB_DIR
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