#!/usr/bin/env bash
# Decrypts objects sealed by the app's unit tests with two independent tools:
#   1. the reference Go implementation `age` (alpine package)
#   2. the server's `sr decrypt`
# Usage: tools/interop-check.sh <dir with *.age + *.bin> [sr image]
set -euo pipefail
DIR="${1:?vector dir}"
SR_IMAGE="${2:-ghcr.io/temoin-app/temoin-server:latest}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ID="$HERE/testdata/identity.txt"
abs() { (cd "$1" && pwd -W 2>/dev/null || pwd); }
D="$(abs "$DIR")"; T="$(abs "$HERE/testdata")"
export MSYS_NO_PATHCONV=1

echo "== reference age (Go) =="
docker run --rm -v "$D:/v" -v "$T:/k:ro" alpine:3.22 sh -ec '
  apk add --no-cache age >/dev/null
  n=0
  for f in /v/*.age; do
    age -d -i /k/identity.txt "$f" > /tmp/pt
    # strip the SRP1 envelope: 4 magic + 4 len + json
    len=$(dd if=/tmp/pt bs=1 skip=4 count=4 2>/dev/null | od -An -tu1 | awk "{print \$1*16777216+\$2*65536+\$3*256+\$4}")
    tail -c +$((9+len)) /tmp/pt > /tmp/content
    cmp -s /tmp/content "${f%.age}.bin" || { echo "MISMATCH $f"; exit 1; }
    n=$((n+1))
  done
  echo "age: $n objects decrypted and match"'

echo "== sr decrypt =="
docker run --rm -v "$D:/v" -v "$T:/k:ro" --entrypoint sr "$SR_IMAGE" decrypt -i /k/identity.txt -d /v -o /tmp/out | tail -3
