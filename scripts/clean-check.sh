#!/usr/bin/env bash
# Checks that a kiln binary works on a system with nothing else installed:
# it runs every conformance program inside an empty (`FROM scratch`)
# container holding only that binary and the programs, and compares the
# output with the expected files (see scripts/lib/conformance.sh). Needs
# Docker.
#
# Usage: scripts/clean-check.sh path/to/kiln
set -euo pipefail

kiln=${1:?usage: scripts/clean-check.sh path/to/kiln}
root=$(cd "$(dirname "$0")/.." && pwd)
image=kiln-clean-check
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

cp "$kiln" "$work/kiln"
cp -r "$root/conformance" "$work/conformance"
cat > "$work/Dockerfile" <<'DOCKERFILE'
FROM scratch
COPY kiln /kiln
COPY conformance /conformance
WORKDIR /work
DOCKERFILE
docker build -q -t "$image" "$work" > /dev/null

source "$root/scripts/lib/conformance.sh"
run_program() {
    local name=$1
    shift
    docker run --rm -i --network none "$image" /kiln run "/conformance/$name" "$@"
}
failed=0
check_conformance || failed=1
docker rmi -f "$image" > /dev/null
exit $failed
