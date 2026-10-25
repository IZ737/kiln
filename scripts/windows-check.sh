#!/usr/bin/env bash
# Checks that a kiln.exe works on Windows with nothing else installed: it
# runs every conformance program with `kiln.exe run` and compares the output
# with the expected files (see scripts/lib/conformance.sh). On Windows (Git
# Bash) it runs kiln.exe directly; elsewhere through Wine (`$WINE`, or
# `wine`).
#
# Usage: scripts/windows-check.sh path/to/kiln.exe
set -euo pipefail

kiln=$(realpath "${1:?usage: scripts/windows-check.sh path/to/kiln.exe}")
root=$(cd "$(dirname "$0")/.." && pwd)
runner=()
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) ;;
    *)
        runner=("${WINE:-wine}")
        export WINEDEBUG=-all
        ;;
esac

cd "$root"
source scripts/lib/conformance.sh
run_program() {
    local name=$1
    shift
    "${runner[@]}" "$kiln" run "conformance/$name" "$@"
}
check_conformance
