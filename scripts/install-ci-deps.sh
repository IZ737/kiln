#!/bin/sh
# GitHub's Azure Ubuntu mirror can stall large MinGW downloads. Use the
# primary Ubuntu archive when the hosted image selects that mirror.
set -eu

if [ "$#" -eq 0 ]; then
    echo "usage: $0 package ..." >&2
    exit 2
fi

if [ -f /etc/apt/apt-mirrors.txt ]; then
    sudo sed -i 's#https\?://azure.archive.ubuntu.com/ubuntu#https://archive.ubuntu.com/ubuntu#g' /etc/apt/apt-mirrors.txt
fi

# Keep signature verification, bounded retries and per-request timeouts.
sudo apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 update
sudo apt-get -o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 install -y "$@"
