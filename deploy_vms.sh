#!/bin/bash
# created by Davide Rovelli
# daft script to deploy source when testing from local machines.
# assumes SSH key auth & DEST directory existing

EXCLUDE_LIST="{'target/','test/','shmem_test.c','deploy.sh','.vscode','.git/','*.iso'}"
DEST="test"
USER=""
NODES=("cxlvm0" "cxlvm1")

# for each node, execute rsync
for node in ${NODES[@]}
do
    echo "Deploying to $node"
    rsync -r ./ $node:$DEST --exclude $EXCLUDE_LIST
done
