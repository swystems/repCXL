#!/usr/bin/env bash
# Parse repCXL YCSB benchmark output files
# Usage: parse_bench.sh <file1.dat> [file2.dat ...]

set -euo pipefail

for file in "$@"; do
    echo "=== $(basename "$file") ==="

    # Monster stats
    if grep -q "Monster stats" "$file"; then
        grep "Monster stats" "$file" | sed 's/.*Monster stats: /Monster: /'
    fi

    # Total ops & throughput
    ops=$(grep "Total operations" "$file" | awk '{print $NF}')
    tp=$(grep "Throughput" "$file" | awk '{print $2, $3}')
    time=$(grep "Total time" "$file" | awk '{print $NF}')
    echo "Ops: $ops  Time: $time  Throughput: $tp"

    # Dirty read percentage
    safe=$(grep "Safe reads" "$file" | awk '{print $NF}')
    dirty=$(grep "Dirty reads" "$file" | awk '{print $NF}')
    pct=$(awk "BEGIN {printf \"%.3f\", 100*$dirty/($safe+$dirty)}")
    echo "Dirty reads: $dirty / $(($safe + $dirty)) ($pct%)"

    # latency distribution vector
    read_vec=$(grep "vec100" "$file" | head -n 1) # read stats
    write_vec=$(grep "vec100" "$file" | tail -n 1) # write stats
    echo "read_lats: $read_vec"
    echo "write_lats: $write_vec"

    # Latency table
    read_avg=$(sed -n '/Read latencies/,/Write latencies/{/avg:/s/.*avg:\t*//p}' "$file")
    read_p99=$(sed -n '/Read latencies/,/Write latencies/{/P99:/s/.*P99:\t*//p}' "$file")
    read_p9999=$(sed -n '/Read latencies/,/Write latencies/{/P99.99:/s/.*P99.99:\t*//p}' "$file")
    read_p100=$(sed -n '/Read latencies/,/Write latencies/{/P100:/s/.*P100:\t*//p}' "$file")

    write_avg=$(sed -n '/Write latencies/,$ {/avg:/s/.*avg:\t*//p}' "$file")
    write_p99=$(sed -n '/Write latencies/,$ {/P99:/s/.*P99:\t*//p}' "$file")
    write_p9999=$(sed -n '/Write latencies/,$ {/P99.99:/s/.*P99.99:\t*//p}' "$file")
    write_p100=$(sed -n '/Write latencies/,$ {/P100:/s/.*P100:\t*//p}' "$file")

    printf "%-7s %12s %12s %12s %12s\n" "" "avg" "P99" "P99.99" "P100"
    printf "%-7s %12s %12s %12s %12s\n" "Read" "$read_avg" "$read_p99" "$read_p9999" "$read_p100"
    printf "%-8s %12s %12s %12s %12s\n" "Write" "$write_avg" "$write_p99" "$write_p9999" "$write_p100"
    echo
done
