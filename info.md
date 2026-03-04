# Useful information

A collection of useful howtos, concepts, explainations. 

## QEMU VM management
> See `ansible/` folder for automated workflows. Info for manual setup below

### create Ubuntu 24.04 VM using CXL memory

Create a disk (at least 20G, will leave 7G available to use with Ubuntu 24.04):

```sh
qemu-img create -f qcow2 cxlvm1_disk.qcow2 20G
```

Get Ubuntu 24.04 server

```sh
wget https://releases.ubuntu.com/noble/ubuntu-24.04.2-live-server-amd64.iso
```

Create VM. The following settings map the CXL host memory (`host-nodes=2` ->
NUMA node 2 where CXL device is attached to) as normal memory device to the VM.

```sh
sudo qemu-system-x86_64 \
-nographic \
-machine q35 \
-cpu host \
-smp 8 \
--enable-kvm \
-object memory-backend-ram,id=cxl-mem,size=16G,host-nodes=2,policy=bind,prealloc=on \
-m 16G,slots=1,maxmem=32G \
-drive if=virtio,file=cxlvm1_disk.qcow2,cache=none \
-net nic,macaddr=52:54:00:12:34:01 \
-net user,hostfwd=tcp::2222-:22 \
-cdrom ubuntu-24.04.2-live-server-amd64.iso
```

In order to see the serial output during Ubuntu installation the `console=ttyS0`
kernel boot option might be needed. Type "e" on the "Try to install Ubuntu" option
in GRUB -> add `console=ttyS0 after "..vmlinuz" word.

### Start a VM

QEMU does is stateless: all emulation option (CPU, mem, NICs) are set at startup.
Only the disk info is retained (OS, files etc.). Start a VM with the same command
as create but without `-cdrom ...`

### Clone / multiple VMs

Copy-paste disk with OS, start VMs with different network interfaces and SSH 
host fowarding ports.


## List of VM configurations

```sh
# Host CXL memory and local RAM
# CXL memory shared via ivshmem as file (no NUMA)
sudo qemu-system-x86_64 \
-machine q35 \
-cpu host \
-smp 8 \
--enable-kvm \
-object memory-backend-file,size=16G,share=on,mem-path=/dev/shm/ivshmem,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
-device ivshmem-plain,memdev=cxl-mem \
-object memory-backend-ram,size=16G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
-numa node,nodeid=0,memdev=local-mem \
-m 16G \
-drive if=virtio,file=cxlvm1_disk.qcow2,cache=none \
-net nic,macaddr=52:54:00:12:34:01 \
-net user,hostfwd=tcp::2222-:22 \
-nographic


# Host CXL memory and local memory on 2 NUMA guest nodes, 16GB + 16GB
sudo qemu-system-x86_64 \
-machine q35 \
-cpu host \
-smp 8 \
--enable-kvm \
-object memory-backend-ram,size=16G,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
-object memory-backend-ram,size=16G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
-numa node,nodeid=0,cpus=0-3,memdev=cxl-mem \
-numa node,nodeid=1,cpus=4-7,memdev=local-mem \
-m 32G,slots=2,maxmem=64G \
-drive if=virtio,file=cxlvm1_disk.qcow2,cache=none \
-net nic,macaddr=52:54:00:12:34:01 \
-net user,hostfwd=tcp::2222-:22 \
-nographic

# Host CXL memory and local memory on 2 NUMA guest nodes, 8GB + 8GB
sudo qemu-system-x86_64 \
-machine q35 \
-cpu host \
-smp 8 \
--enable-kvm \
-object memory-backend-ram,size=8G,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
-object memory-backend-ram,size=8G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
-numa node,nodeid=0,cpus=4-7,memdev=cxl-mem \
-numa node,nodeid=1,cpus=0-3,memdev=local-mem \
-m 16G,slots=2,maxmem=32G \
-drive if=virtio,file=cxlvm1_disk.qcow2,cache=none \
-net nic,macaddr=52:54:00:12:34:01 \
-net user,hostfwd=tcp::2222-:22 \
-nographic

# Host CXL memory on guest local node
sudo qemu-system-x86_64 \
-machine q35 \
-cpu host \
-smp 50 \
--enable-kvm \
-object memory-backend-ram,size=16G,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
-m 16G,slots=1,maxmem=32G \
-numa node,nodeid=0,memdev=cxl-mem \
-drive if=virtio,file=cxlvm2_disk.qcow2,cache=none \
-net nic,macaddr=52:54:00:12:34:02 \
-net user,hostfwd=tcp::2223-:22 \
-nographic

# Local, remote and CXL memory as NUMA (same config as host)
sudo qemu-system-x86_64 \
-machine q35 \
-cpu host \
-smp 33 \
--enable-kvm \
-object memory-backend-ram,size=16G,host-nodes=0,policy=bind,prealloc=on,id=local-mem \
-object memory-backend-ram,size=16G,host-nodes=1,policy=bind,prealloc=on,id=remote-mem \
-object memory-backend-ram,size=16G,host-nodes=2,policy=bind,prealloc=on,id=cxl-mem \
-m 48G,slots=3,maxmem=64G \
-numa node,nodeid=0,memdev=local-mem \
-numa node,nodeid=1,memdev=remote-mem \
-numa node,nodeid=2,memdev=cxl-mem \
-drive if=virtio,file=cxlvm1_disk.qcow2,cache=none \
-net nic,macaddr=52:54:00:12:34:02 \
-net user,hostfwd=tcp::2222-:22 \
-nographic
```