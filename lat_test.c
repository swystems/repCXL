#include <stdio.h>
#include <fcntl.h>
#include <sys/mman.h>
#include <unistd.h>
// #include <unistd.h>
#include <time.h>

// Change this to a physical address range found in /proc/iomem
// CAUTION: Ensure this isn't active kernel memory!
#define PHYS_ADDR 0x160000000
#define MAP_SIZE (10 * 1024 * 1024) // 10MiB


static inline unsigned long long rdtsc() {
    unsigned int lo, hi;
    __asm__ __volatile__ ("rdtsc" : "=a" (lo), "=d" (hi));
    return ((unsigned long long)hi << 32) | lo;
}


int main() {
    // Using the 'resource2' file for the BAR containing the shared memory
    int fd = open("/sys/bus/pci/devices/0000:00:03.0/resource2", O_RDWR | O_SYNC);
    // int fd = open("/dev/mem", O_RDWR | O_SYNC);
    
    if (fd < 0) {
        perror("Failed to open PCI resource. Are you root?");
        return 1;
    }

    // Map 128MB
    void *map_ptr = mmap(NULL, MAP_SIZE, PROT_READ | PROT_WRITE, MAP_SHARED, fd, PHYS_ADDR);
    
    // Because we used O_SYNC on a PCI BAR, the CPU will access this 
    // as UNCACHED (UC) automatically via the Page Attribute Table.
    
    volatile unsigned int *data = (volatile unsigned int *)map_ptr;
    
    unsigned long long start = rdtsc();
    for(int i = 0; i < 10000; i++) {
        data[0] = i; // UC write
    }
    unsigned long long end = rdtsc();
    printf("Avg Uncached Write Latency: %llu cycles\n", (end - start) / 1000);

    // 5. Measure Read Latency
    start = rdtsc();
    for(int i = 0; i < 10000; i++) {
        unsigned long val = data[0]; // UC read
        (void)val;
    }
    end = rdtsc();
    printf("Avg Uncached Read Latency: %llu cycles\n", (end - start) / 1000);

    munmap(map_ptr, MAP_SIZE);
    close(fd);
    return 0;
}