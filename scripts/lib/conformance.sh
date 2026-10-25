# Runs every conformance program and compares its output with the expected
# files, as crates/kiln_cli/tests/conformance.rs does. Sourced by the check
# scripts, which define how a program is run:
#
#   run_program NAME ARGS...   runs conformance/NAME with ARGS; standard
#                              input, output and error are redirected
#
# Then `check_conformance` prints one line per program and returns 1 if any
# failed.

check_conformance() {
    local root dir name input code expected_code failed=0
    root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
    local work
    work=$(mktemp -d)
    for dir in "$root"/conformance/*/; do
        name=$(basename "$dir")
        local args=()
        if [[ -f $dir/main.args ]]; then
            mapfile -t args < "$dir/main.args"
        fi
        input=/dev/null
        [[ -f $dir/main.in ]] && input=$dir/main.in
        set +e
        run_program "$name" "${args[@]}" < "$input" > "$work/out" 2> "$work/err"
        code=$?
        set -e
        expected_code=0
        if [[ -f $dir/main.code ]]; then
            expected_code=$(tr -d '[:space:]' < "$dir/main.code")
        elif [[ -f $dir/main.err ]]; then
            if tail -n 1 "$dir/main.err" | grep -q '^panic:'; then expected_code=101; else expected_code=1; fi
        fi
        local problems=()
        cmp -s "$work/out" <(cat "$dir/main.out" 2> /dev/null) || problems+=("standard output differs: $(diff "$work/out" <(cat "$dir/main.out" 2> /dev/null) | head -c 300 || true)")
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
    rm -rf "$work"
    return $failed
}
