#!/bin/bash
# created by Davide Rovelli
# daft script to deploy source when testing from local machines.
# assumes SSH key auth & DEST directory existing

EXCLUDE_LIST="{'test/','shmem_test.c','deploy.sh','example.asgard-bench.ini','.vscode','.config.hash'}"
DEST="dmem-test"
USER=""
NODES=("cxlvm1" "cxlvm2")

# for each node, execute rsync
for node in ${NODES[@]}
do
    echo "Deploying to $node"
    rsync -r ./ $node:$DEST --exclude $EXCLUDE_LIST
done
