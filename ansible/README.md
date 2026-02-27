# VM setup

## Requirements

- ansible 

```sh
pipx install ansible ansible-core
pipx ensurepath
# some required collections
ansible-galaxy collection install ansible.posix
```

- 16GB memory (change the parameters in `cxl_vm.sh` otherwise)
- CXL memory mapped as numa node


## howto

### Create a VM in localhost with ubuntu 24.04 and CXL memory mapped to NUMA node 0. 

    cd ansible 
    ansible-playbook -i inv/local.yml vm.create.yml -K -e vm_id=0

and enter sudo password.

The VM starts in the background and its disk is created in `$HOME/repCXL-ansible/vm<vm_id>.qcow2`

### Create another VM

Change `vm_id`

    ansible-playbook -i inv/local.ini vm.create.yml -K -e vm_id=1

### Force the re-creation of a VM
Make sure all VMs are stopped and add `-e force=true`

```sh
pkill qemu # on the host machine
```

```sh
# locally
ansible-playbook -i inv/local.ini vm.create.yml -K -e vm_id=1 -e force=true
```

### Provision all VMs

Edit inventory with the ports and hostnames of the running VMs then

    ansible-playbook -i inv/local.ini vms.provision.yml -K

### Clone and test this repo on all VMs

Edit inventory with the ports and hostnames of the running VMs then

    ansible-playbook -i inv/local.ini vms.deploy.yml -K

### SSH

    ssh ansible@localhost -p 222<vm_id> -i ~/.ssh/ansible_key

### VM lifecycle post-creation

Stop VMs
```sh
pgrep -af repcxl-vm # get VM pids
kill <pid>          # kill specific vm
pkill repcxl-vm     # stop all repCXL vms
```

Restart VMs (no need to run ansible again)
```sh
    cd $HOME/repCXL-ansible
    ./qemu_cxl.sh <vm_id> <cxl_numa_node>
```

### Create a VM on a remote server

Create another ansible inventory in `inv/remote.ini` with for remote servers info instead
of localhost (requires passwordless SSH access). Run

    ansible-playbook -i inv/remote.ini -K -e vm_id=0

### Find PCI devices mapped to shared (possibly CXL) memory from a VM

QEMU maps shared memory to the VMs as ivmshmem devices, which correspond to PCIe
devices. Run the following to get them

    >lspci | grep Inter-VM
    00:03.0 RAM memory: Red Hat, Inc. Inter-VM shared memory (rev 01)


The corresponding memory region is in `resource2`, in the above case

    /sys/bus/pci/devices/0000:00:03.0/resource2

