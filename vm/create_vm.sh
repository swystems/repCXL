#!/usr/bin/env bash
set -e

IMAGE_URL="https://cloud-images.ubuntu.com/noble/current/noble-server-cloudimg-amd64.img"
BASE_IMAGE="ubuntu2404-cloudimg-noble.img"
VM_DISK="vm.qcow2"
SEED_ISO="seed.iso"

if [ ! -f "$BASE_IMAGE" ]; then
    echo "Downloading Ubuntu cloud image..."
    wget -O $BASE_IMAGE $IMAGE_URL
fi

echo "Creating overlay disk..."
qemu-img create -f qcow2 -b $BASE_IMAGE $VM_DISK 20G

echo "Creating cloud-init seed..."
cloud-localds $SEED_ISO cloud-init/user_data.yaml cloud-init/meta_data.yaml

echo "Starting VM..."
qemu-system-x86_64 \
    -machine q35 \
    -cpu host \
    -m 32G,slots=2,maxmem=64G \
    -smp 8 \
    --enable-kvm \
    -object memory-backend-ram,size=16G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
    -object memory-backend-file,size=16M,share=on,mem-path=/dev/shm/ivshmem,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
    -device ivshmem-plain,memdev=cxl-mem \
    -drive file=$VM_DISK,format=qcow2 \
    -drive file=$SEED_ISO,format=raw \
    -net nic,macaddr=52:54:00:12:34:01 -net user,hostfwd=tcp::2222-:22 \
    -daemonize \
    -display none


