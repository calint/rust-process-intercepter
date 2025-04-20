#!/bin/sh
set -e
cd $(dirname "$0")

cargo build
target/debug/rust-process-intercepter ../rust_rv32i_os/scripts/emulator-run.sh
