#!/bin/bash
# A fuzz target's throughput against its floor (the CI fuzz job's
# "Throughput" step): its rate over the run, libFuzzer's last line
# "Done N runs in T second(s)" as N / T, said with the floor wanted; under
# the floor, or with no such line or no floor, it fails.
#   bash fuzz/throughput.sh <target> <libFuzzer's output> <floors file>
set -u
target=$1
log=$2
floors=$3
floor=$(awk -v t="$target" '$1 == t {print $2}' "$floors")
if [ -z "$floor" ]; then
  echo "::error::no throughput floor for $target in $floors"
  exit 1
fi
done_line=$(grep -oE 'Done [0-9]+ runs in [0-9]+ second' "$log" | tail -1)
if [ -z "$done_line" ]; then
  echo "::error::$target: no 'Done N runs in T second(s)' line in $log"
  exit 1
fi
awk -v line="$done_line" -v f="$floor" -v name="$target" 'BEGIN {
  split(line, w, " ")
  n = w[2]; t = w[5]
  rate = n / (t > 0 ? t : 1)
  printf "%s: %.3f runs a second (%d in %d s), wanted at least %s\n", name, rate, n, t, f
  if (rate < f) {
    printf "::error::%s ran %.3f runs a second, under its floor of %s\n", name, rate, f
    exit 1
  }
}'
