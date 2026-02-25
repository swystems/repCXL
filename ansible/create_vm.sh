#!/usr/bin/env bash
# Script to bootstrap a QEMU VM with local memory (numa node 0) and CXL memory
# Assmues CXL memory is exposed as NUMA node
#
# Args:
# $1: VM ID (used for disk and port naming)
# $2: NUMA node number where CXL memory is located on the host
# 
# VM starts as deamon, see below for VM and CXL memory settings 
set -e

VM_ID=$1
NUMA_NODE_CXL=$2

if [ -z "$VM_ID" ] || [ -z "$NUMA_NODE_CXL" ]; then
  echo "Usage: $0 <vm_id> <numa_node_cxl>"
  exit 1
fi

#create 1GiB of shared mem which will be mapped to CXL node
truncate -s 1G /dev/shm/ivshmem
chmod 666 /dev/shm/ivshmem

echo "Starting VM..."
qemu-system-x86_64 \
    -machine q35 \
    -cpu host \
    -m 8G,slots=2,maxmem=16G \
    -smp 4 \
    --enable-kvm \
    -object memory-backend-ram,size=8G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
    -object memory-backend-file,size=1G,share=on,mem-path=/dev/shm/ivshmem,host-nodes=${NUMA_NODE_CXL},policy=bind,prealloc=on,id=cxl-mem \
    -device ivshmem-plain,memdev=cxl-mem \
    -drive file=vm${VM_ID}.qcow2,format=qcow2 \
    -drive file=seed.iso,format=raw \
    -net nic -net user,hostfwd=tcp::222${VM_ID}-:22 \
    -daemonize \
    -display none


