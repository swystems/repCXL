
for d in ./*; do
  if [ -d "$d" ]; then
    tput=$(grep tput $d/* | awk '{sum += $3} END {print sum/NR}')
    echo "$d: $tput kops/s"
  fi
done
