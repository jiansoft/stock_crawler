#! /bin/bash

#export CC_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-gcc
#export CXX_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-g++
#export AR_x86_64_unknown_linux_gnu=x86_64-unknown-linux-gnu-ar
#export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-unknown-linux-gnu-gcc

export CC=aarch64-linux-gnu-gcc
export CXX=aarch64-linux-gnu-g++
export AR=aarch64-linux-gnu-ar
export LD=aarch64-linux-gnu-ld

#export OPENSSL_DIR=/usr/local/Cellar/openssl@3/3.0.7/bin/
#export OPENSSL_INCLUDE_DIR=/usr/local/Cellar/openssl@3/3.0.7/include/
#export OPENSSL_LIB_DIR=/usr/local/Cellar/openssl@3/3.0.7/lib/
cargo build --target aarch64-unknown-linux-gnu --release
#cargo build --target x86_64-unknown-linux-gnu --release
#cargo build --target x86_64-pc-windows-gnu --release