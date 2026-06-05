
for d in ./*; do
  if [ -d "$d" ]; then
    tput=$(grep tput $d/* | awk '{sum += $3} END {print sum/NR}')
    echo "$d: $tput kops/s"

    ## output files have
    ## SEARCH 
    ## ..
    ## Average latency: 3756ns
    ## GET ..
    ## UPDATE ..
    ##
    ## we want GET UPDATE stats (not SEARCH) = last 213 lines
    ## filter for "Average latency" 
    ## Average the average get and update latencies
    avglat=$(tail -n 213 $d/* | grep "Average latency" | awk '{sum += $3} END {print sum/NR}')
    echo "$d: $avglat ns"
  fi
done
