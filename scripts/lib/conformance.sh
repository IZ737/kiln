# Runs every conformance program and compares its output with the expected
# files, as crates/kiln_cli/tests/conformance.rs does. Sourced by the check
# scripts, which define how a program is run:
#
#   run_program NAME ARGS...   runs conformance/NAME with ARGS; standard
#                              input, output and error are redirected, and
#                              PORT, when it is set, is passed on
#
# Then `check_conformance` prints one line per program and returns 1 if any
# failed. A program with `main.requests` is a server: it runs in the
# background with a port in PORT, and `send_requests` sends it the requests
# through bash's /dev/tcp.

# Splits main.requests into one file per request, as the HTTP client in
# conformance.rs sends them: lines in CRLF, content-length added when there
# is a body, and `connection: close` unless the request names a connection.
split_requests() {
    LC_ALL=C awk -v dir="$2" '
        function flush(   i, raw, has_len, chunked, has_conn, body) {
            if (n == 0) return
            raw = ""; has_len = 0; chunked = 0; has_conn = 0; body = ""
            for (i = 1; i <= head; i++) {
                raw = raw lines[i] "\r\n"
                if (tolower(lines[i]) ~ /^connection:/) has_conn = 1
                if (tolower(lines[i]) ~ /^(content-length|transfer-encoding):/) has_len = 1
                if (tolower(lines[i]) ~ /^transfer-encoding: chunked/) chunked = 1
            }
            for (i = head + 2; i <= n; i++) body = body (i > head + 2 ? (chunked ? "\r\n" : "\n") : "") lines[i]
            sub(/[\r\n]+$/, "", body)
            if (chunked) body = body "\r\n\r\n"
            else if (body != "" && !has_len) raw = raw "content-length: " length(body) "\r\n"
            if (!has_conn) raw = raw "connection: close\r\n"
            count++
            printf "%s\r\n%s", raw, body > (dir "/" count)
            close(dir "/" count)
            n = 0; head = 0; inbody = 0
        }
        /^###$/ { flush(); next }
        {
            if (n == 0 && $0 == "") next
            lines[++n] = $0
            if (!inbody) { if ($0 == "") inbody = 1; else head = n }
        }
        END { flush() }
    ' "$1"
}

# Sends the requests in $1 to 127.0.0.1:$2 once it accepts connections,
# writing the responses as main.responses holds them.
send_requests() {
    local file=$1 port=$2 dir i tries=0
    dir=$(mktemp -d)
    split_requests "$file" "$dir"
    until (exec 3<> "/dev/tcp/127.0.0.1/$port") 2> /dev/null; do
        tries=$((tries + 1))
        if ((tries > 600)); then
            echo "the server never accepted a connection"
            rm -rf "$dir"
            return
        fi
        sleep 0.1
    done
    for ((i = 1; ; i++)); do
        [[ -f $dir/$i ]] || break
        printf '### %s\n' "$(head -n 1 "$dir/$i" | tr -d '\r')"
        if exec 3<> "/dev/tcp/127.0.0.1/$port"; then
            cat "$dir/$i" >&3
            cat <&3 | tr -d '\r' | grep -iv '^date:' || true
            exec 3>&-
        fi
    done
    rm -rf "$dir"
}

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
        if [[ -f $dir/main.requests ]]; then
            local port=$((20000 + RANDOM % 30000)) pid waited=0
            PORT=$port run_program "$name" "${args[@]}" < "$input" > "$work/out" 2> "$work/err.raw" &
            pid=$!
            send_requests "$dir/main.requests" "$port" > "$work/responses"
            # The last request should have ended the program.
            while kill -0 "$pid" 2> /dev/null && ((waited < 300)); do
                sleep 0.1
                waited=$((waited + 1))
            done
            kill "$pid" 2> /dev/null
            wait "$pid"
            code=$?
            sed "s/:$port/:PORT/g" "$work/err.raw" > "$work/err"
        else
            run_program "$name" "${args[@]}" < "$input" > "$work/out" 2> "$work/err"
            code=$?
        fi
        set -e
        expected_code=0
        if [[ -f $dir/main.code ]]; then
            expected_code=$(tr -d '[:space:]' < "$dir/main.code")
        elif [[ -f $dir/main.requests ]]; then
            expected_code=0
        elif [[ -f $dir/main.err ]]; then
            if tail -n 1 "$dir/main.err" | grep -q '^panic:'; then expected_code=101; else expected_code=1; fi
        fi
        local problems=()
        cmp -s "$work/out" <(cat "$dir/main.out" 2> /dev/null) || problems+=("standard output differs: $(diff "$work/out" <(cat "$dir/main.out" 2> /dev/null) | head -c 300 || true)")
        cmp -s "$work/err" <(cat "$dir/main.err" 2> /dev/null) || problems+=("standard error differs: $(head -c 300 "$work/err")")
        if [[ -f $dir/main.requests ]]; then
            cmp -s "$work/responses" "$dir/main.responses" || problems+=("responses differ: $(diff "$work/responses" "$dir/main.responses" | head -c 600 || true)")
        fi
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
