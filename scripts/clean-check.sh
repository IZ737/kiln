#!/usr/bin/env bash
# Checks that a kiln binary works on a system with nothing else installed:
# it runs every conformance program inside an empty (`FROM scratch`)
# container holding only that binary and the programs, and compares the
# output with the expected files, as crates/kiln_cli/tests/conformance.rs
# does. Needs Docker.
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

failed=0
for dir in "$root"/conformance/*/; do
    name=$(basename "$dir")
    args=()
    if [[ -f $dir/main.args ]]; then
        mapfile -t args < "$dir/main.args"
    fi
    input=/dev/null
    [[ -f $dir/main.in ]] && input=$dir/main.in
    set +e
    docker run --rm -i --network none "$image" /kiln run "/conformance/$name" "${args[@]}" \
        < "$input" > "$work/out" 2> "$work/err"
    code=$?
    set -e
    expected_code=0
    if [[ -f $dir/main.code ]]; then
        expected_code=$(tr -d '[:space:]' < "$dir/main.code")
    elif [[ -f $dir/main.err ]]; then
        if tail -n 1 "$dir/main.err" | grep -q '^panic:'; then expected_code=101; else expected_code=1; fi
    fi
    problems=()
    cmp -s "$work/out" <(cat "$dir/main.out" 2> /dev/null) || problems+=("standard output differs")
    cmp -s "$work/err" <(cat "$dir/main.err" 2> /dev/null) || problems+=("standard error differs: $(head -c 300 "$work/err")")
    [[ $code == "$expected_code" ]] || problems+=("exit code $code, expected $expected_code")
    if ((${#problems[@]})); then
        failed=1
        printf 'FAIL %s\n' "$name"
        printf '  %s\n' "${problems[@]}"
    else
        printf 'ok   %s\n' "$name"
    fi
done
docker rmi -f "$image" > /dev/null
exit $failed
