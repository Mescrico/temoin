#!/usr/bin/env bash
# Decrypts every *.age in DIR with the reference `age` CLI and compares the
# envelope content with the matching *.bin. Usage: tools/check-vectors.sh DIR
set -euo pipefail
DIR="${1:?vector dir}"
ID="$(cd "$(dirname "$0")" && pwd)/testdata/identity.txt"
n=0
for f in "$DIR"/*.age; do
  age -d -i "$ID" "$f" | python3 -c '
import sys, struct
pt = sys.stdin.buffer.read()
assert pt[:4] == b"SRP1", "bad magic"
(n,) = struct.unpack(">I", pt[4:8])
sys.stdout.buffer.write(pt[8 + n:])' | cmp - "${f%.age}.bin"
  n=$((n + 1))
done
[ "$n" -gt 0 ] || { echo "no vectors in $DIR"; exit 1; }
echo "$n vectors decrypt with the reference age and match"
