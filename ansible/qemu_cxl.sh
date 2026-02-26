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
MEMORY_NODES=$3

if [ -z "$VM_ID" ] || [ -z "$NUMA_NODE_CXL" ] || [ -z "$MEMORY_NODES" ]; then
  echo "Usage: $0 <vm_id> <numa_node_cxl> <memory_nodes>"
  exit 1
fi

QEMU_ARGS=(
    -name "repcxl-vm${VM_ID}"
    -machine q35
    -cpu host
    -m 8G,slots=2,maxmem=16G
    -smp 4
    --enable-kvm
    -object "memory-backend-ram,size=8G,host-nodes=0,policy=bind,prealloc=on,id=local-mem"
    -drive "file=vm${VM_ID}.qcow2,format=qcow2"
    -net nic -net "user,hostfwd=tcp::222${VM_ID}-:22"
    -daemonize
    -display none
)

# create 1GiB shared memory files for ivshmem which will be mapped to the same
# CXL memory region to simulate multiple memory nodes
for node in $(seq 0 $((MEMORY_NODES-1))); do
    # create shared memory file for ivshmem
    truncate -s 1G /dev/shm/ivshmem${node}
    chmod 666 /dev/shm/ivshmem${node}
    # QEMU map options
    QEMU_ARGS+=(
        -object "memory-backend-file,size=1G,share=on,mem-path=/dev/shm/ivshmem${node},host-nodes=${NUMA_NODE_CXL},policy=bind,prealloc=on,id=cxl-mem${node}"
        -device ivshmem-plain,memdev=cxl-mem${node}
    )
done

# attach cloud-init seed on first boot
if [[ "${FIRST_BOOT:-0}" -eq 1 ]]; then
    QEMU_ARGS+=(-drive "file=seed${VM_ID}.iso,format=raw")
fi

echo "Starting VM ${VM_ID}..."
qemu-system-x86_64 "${QEMU_ARGS[@]}"