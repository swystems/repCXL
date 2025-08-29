#!/bin/bash
# QEMU VM with CXL memory attached as default and only memory.
# Assmues CXL memory on the host is on NUMA node 2
# VMs start in background
# assumes vms/ disks and this repo to be in HOME_FOLDER. See below

source .env

# echo $HOME_FOLDER
if [ -z "$HOME_FOLDER" ]; then
  echo "HOME_FOLDER is not set. Please set it in the .env file."
  exit 1
fi
VM1_DISK="${HOME_FOLDER}/vms/ubuntu2404_1.qcow2"
VM2_DISK="${HOME_FOLDER}/vms/ubuntu2404_2.qcow2"
HOST_VM_FOLDER="${HOME_FOLDER}/repCXL"

#create 16M of shared disk which will be mapped to CXL node
truncate -s 16M /dev/shm/ivshmem
chmod 666 /dev/shm/ivshmem

COMMON_SETTINGS="-machine q35 \
-cpu host \
-smp 8 \
--enable-kvm \
-object memory-backend-ram,size=16G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
-object memory-backend-file,size=16M,share=on,mem-path=/dev/shm/ivshmem,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
-device ivshmem-plain,memdev=cxl-mem \
-m 32G,slots=2,maxmem=64G \
-daemonize \
-virtfs local,path=$HOST_VM_FOLDER,mount_tag=host-folder,security_model=mapped-xattr \
-display none"

# VM1
sudo qemu-system-x86_64 \
-drive if=virtio,file=$VM1_DISK,cache=none \
-net nic,macaddr=52:54:00:12:34:01 \
-net user,hostfwd=tcp::2222-:22 \
${COMMON_SETTINGS} &

# VM2
sudo qemu-system-x86_64 \
-drive if=virtio,file=$VM2_DISK,cache=none \
-net nic,macaddr=52:54:00:12:34:02 \
-net user,hostfwd=tcp::2223-:22 \
${COMMON_SETTINGS}
