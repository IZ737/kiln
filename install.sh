#!/bin/sh
# Installs the latest kiln release, or the one you name, for the current
# user:
#
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/IZ737/kiln/main/install.sh | sh
#   curl --proto '=https' --tlsv1.2 -sSf https://raw.githubusercontent.com/IZ737/kiln/main/install.sh | sh -s -- --version v0.2.0
#
# It downloads the release archive for this system, checks it against the
# release's .sha256 file, puts `kiln` in ~/.kiln/bin and adds that folder to
# PATH in your shell's startup files. `sh install.sh --help` lists the
# options.

# Everything is inside main, called on the last line, so a download cut off
# halfway runs nothing.

set -u

REPO="IZ737/kiln"

usage() {
    cat <<EOF
Installs kiln, the Kiln compiler.

Usage: install.sh [options]

Options:
  --version <tag>     Install this release, such as v0.2.0 (default: the latest)
  --dir <path>        Install into <path>/bin (default: \$KILN_HOME or ~/.kiln)
  --no-modify-path    Do not add kiln to PATH in your shell's startup files
  -y, --yes           Accepted for compatibility; the script never asks
  -h, --help          Show this help

The environment variables KILN_VERSION, KILN_HOME and KILN_NO_MODIFY_PATH=1
do the same as the options.
EOF
}

say() {
    printf 'kiln: %s\n' "$1"
}

err() {
    printf 'kiln: error: %s\n' "$1" >&2
    exit 1
}

need() {
    command -v "$1" > /dev/null 2>&1 || err "this script needs '$1', and it was not found"
}

# GNU wget can refuse redirects away from https; BusyBox's (on Alpine) cannot.
wget_opts() {
    wget --help 2>&1 | grep -q -- --https-only && printf '%s' --https-only
}

# download <url> <file>: fails quietly on a missing file (HTTP 404), so the
# caller can say what was missing.
download() {
    if command -v curl > /dev/null 2>&1; then
        curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
    elif command -v wget > /dev/null 2>&1; then
        wget $(wget_opts) -q "$1" -O "$2"
    else
        err "this script needs curl or wget, and neither was found"
    fi
}

# The latest release's tag, from where github.com/<repo>/releases/latest
# redirects. This needs no API token and has no rate limit.
latest_version() {
    _url="https://github.com/$REPO/releases/latest"
    if command -v curl > /dev/null 2>&1; then
        _final=$(curl --proto '=https' --tlsv1.2 -fsSLI -o /dev/null -w '%{url_effective}' "$_url") || return 1
    else
        _final=$(wget $(wget_opts) -q -S --spider "$_url" 2>&1 | sed -n 's/^ *[Ll]ocation: *//p' | tail -n 1 | tr -d '\r') || return 1
    fi
    _tag=${_final##*/}
    case "$_tag" in
        v[0-9]*) printf '%s\n' "$_tag" ;;
        *) return 1 ;;
    esac
}

sha256_of() {
    if command -v sha256sum > /dev/null 2>&1; then
        sha256sum "$1" | cut -d ' ' -f 1
    elif command -v shasum > /dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d ' ' -f 1
    elif command -v openssl > /dev/null 2>&1; then
        openssl dgst -sha256 "$1" | sed 's/^.*= *//'
    else
        err "this script needs sha256sum, shasum or openssl to check the download"
    fi
}

# Appends `line` to `file` unless the file already has it.
add_line() {
    _file=$1
    _line=$2
    if [ -f "$_file" ] && grep -qF "$_line" "$_file"; then
        return 0
    fi
    mkdir -p "$(dirname "$_file")"
    printf '\n%s\n' "$_line" >> "$_file"
    say "added kiln to PATH in $_file"
}

modify_path() {
    _bin=$1
    _home=$2

    # One env file that the startup files source, as rustup does with
    # ~/.cargo/env, so PATH is set the same way everywhere and only once.
    cat > "$_home/env" <<EOF
# Added by the kiln installer: puts kiln on PATH.
case ":\${PATH}:" in
    *:"$_bin":*) ;;
    *) export PATH="$_bin:\$PATH" ;;
esac
EOF
    _source=". \"$_home/env\""

    # Login shells read ~/.profile (or bash's ~/.bash_profile when it
    # exists); interactive bash reads ~/.bashrc; zsh reads ~/.zshenv.
    add_line "$HOME/.profile" "$_source"
    [ -f "$HOME/.bash_profile" ] && add_line "$HOME/.bash_profile" "$_source"
    [ -f "$HOME/.bash_login" ] && add_line "$HOME/.bash_login" "$_source"
    if [ -f "$HOME/.bashrc" ] || command -v bash > /dev/null 2>&1; then
        add_line "$HOME/.bashrc" "$_source"
    fi
    if [ -f "$HOME/.zshenv" ] || command -v zsh > /dev/null 2>&1; then
        add_line "${ZDOTDIR:-$HOME}/.zshenv" "$_source"
    fi
    if command -v fish > /dev/null 2>&1 || [ -d "$HOME/.config/fish" ]; then
        _fish="${XDG_CONFIG_HOME:-$HOME/.config}/fish/conf.d/kiln.fish"
        mkdir -p "$(dirname "$_fish")"
        printf '# Added by the kiln installer: puts kiln on PATH.\nfish_add_path --path --prepend "%s"\n' "$_bin" > "$_fish"
        say "added kiln to PATH in $_fish"
    fi
}

main() {
    version=${KILN_VERSION:-}
    home=${KILN_HOME:-$HOME/.kiln}
    modify=1
    [ "${KILN_NO_MODIFY_PATH:-0}" = 1 ] && modify=0

    while [ $# -gt 0 ]; do
        case "$1" in
            --version)
                [ $# -ge 2 ] || err "--version needs a release, such as v0.2.0"
                version=$2
                shift 2
                ;;
            --version=*) version=${1#--version=}; shift ;;
            --dir)
                [ $# -ge 2 ] || err "--dir needs a folder"
                home=$2
                shift 2
                ;;
            --dir=*) home=${1#--dir=}; shift ;;
            --no-modify-path) modify=0; shift ;;
            -y | --yes) shift ;;
            -h | --help) usage; exit 0 ;;
            *) err "unknown option '$1' (see --help)" ;;
        esac
    done

    need uname
    need tar
    need mkdir
    need mktemp

    os=$(uname -s)
    arch=$(uname -m)
    case "$os" in
        Linux) os=linux ;;
        Darwin) os=macos ;;
        MINGW* | MSYS* | CYGWIN* | Windows_NT)
            err "on Windows, install kiln from PowerShell instead: irm https://raw.githubusercontent.com/$REPO/main/install.ps1 | iex"
            ;;
        *) err "there is no prebuilt kiln for $os; build it from source: https://github.com/$REPO#try-it" ;;
    esac
    case "$arch" in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) err "there is no prebuilt kiln for $arch processors; build it from source: https://github.com/$REPO#try-it" ;;
    esac
    # A 64-bit Mac running this shell under Rosetta still reports x86_64,
    # which is right: Rosetta runs the x86_64 build.

    if [ -z "$version" ]; then
        version=$(latest_version) || err "could not find the latest release at https://github.com/$REPO/releases"
    fi
    case "$version" in
        v*) ;;
        *) version="v$version" ;;
    esac

    name="kiln-$version-$arch-$os"
    url="https://github.com/$REPO/releases/download/$version/$name.tar.gz"

    tmp=$(mktemp -d 2> /dev/null || mktemp -d -t kiln) || err "could not make a temporary folder"
    trap 'rm -rf "$tmp"' EXIT
    trap 'exit 1' INT TERM

    say "downloading kiln $version for $os $arch"
    if ! download "$url" "$tmp/$name.tar.gz"; then
        # Tell a missing release apart from a release without this system.
        if download "https://github.com/$REPO/releases/tag/$version" "$tmp/release.html" 2> /dev/null; then
            err "release $version has no prebuilt kiln for $os $arch; build it from source: https://github.com/$REPO#try-it"
        fi
        err "could not download release $version; the releases are at https://github.com/$REPO/releases"
    fi
    download "$url.sha256" "$tmp/$name.tar.gz.sha256" || err "could not download $url.sha256"

    expected=$(cut -d ' ' -f 1 < "$tmp/$name.tar.gz.sha256")
    actual=$(sha256_of "$tmp/$name.tar.gz")
    if [ -z "$expected" ] || [ "$expected" != "$actual" ]; then
        err "the download does not match its checksum (expected $expected, got $actual); try again"
    fi

    tar xzf "$tmp/$name.tar.gz" -C "$tmp" || err "could not unpack $name.tar.gz"
    [ -f "$tmp/$name/kiln" ] || err "$name.tar.gz has no kiln in it"

    bin="$home/bin"
    mkdir -p "$bin" || err "could not make $bin"
    # Copy then rename, so a running kiln is replaced rather than changed.
    cp "$tmp/$name/kiln" "$bin/kiln.new" && chmod 755 "$bin/kiln.new" && mv -f "$bin/kiln.new" "$bin/kiln" ||
        err "could not install kiln into $bin"
    say "installed kiln $version to $bin/kiln"

    if [ "$modify" = 1 ]; then
        modify_path "$bin" "$home"
    fi

    case ":$PATH:" in
        *:"$bin":*) say "run 'kiln --help' to get started" ;;
        *)
            if [ "$modify" = 1 ]; then
                say "open a new terminal, or run '. \"$home/env\"', to use kiln"
            else
                say "add $bin to PATH to use kiln"
            fi
            ;;
    esac
}

main "$@"
