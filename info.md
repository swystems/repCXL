# Useful information

A collection of useful howtos, concepts, explainations. 

## Run tests

Unit tests: test internal RepCXL functions

    cargo test

Memory test: access latency of a memory-backed file. Examples:

```sh
cargo build --release
target/debug/mem_test --help # usage and help
# mem test on CXL dax device with 100k iterations and object size of 128B
target/debug/mem_test /dev/dax0.0 -o 128 -n 100000
```

Shared memory test: verify different RepCXL intances correctly communicate through shared memory. 

```sh
# coordinator first (sets up the memory)
target/release/shmem_obj_test coordinator -c config/ansible.toml
# then replica
target/release/shmem_obj_test replica -c config/ansible.toml
sudo target/release/shmem_obj_test replica -c config/ansible.toml
DEBUG [rep_cxl::safe_memio] write_size: 32B, step 1: 310 step2: 560, step3: 320
Replica successfully read: Hello, RepCXL!
INFO  [rep_cxl] Stopping repCXL process 0. Goodbye...
```

## Create CXL DAX devices

CXL memory can be attached either as (1) a NUMA node or as (2) an mmappable DAX 
character device. We can use (1) for memory tests like `mlc` when we want the 
kernel to manage the memory for us. (2) is more convenient when we want to manage
the memory ourselves, e.g., for repCXL shared memory use case. 

Create multiple multiple DAX devices (to simulate memory nodes) as follows. 
Load necessary modules:

```sh
echo offline > /sys/devices/system/memory/auto_online_blocks

modprobe cxl_pci
modprobe cxl_mem
modprobe cxl_acpi
modprobe cxl_pmem
modprobe cxl_port
modprobe dax_hmem
modprobe device_dax
```

List existing dax devices:

    daxctl list -u

List available CXL region:

    daxctl list -Ru

Create a new device with 4G memory

    daxctl create-device -s 4G

If an existing device fills all the available memory already, resize

    daxctl reconfigure-device -s <new_size> -u /dev/dax<number>


## QEMU VM management
> See `ansible/` folder for automated workflows. Info for manual setup below

### create Ubuntu 24.04 VM using CXL memory

Create a disk (at least 20G, will leave 7G available to use with Ubuntu 24.04):
rcxl.write_object
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