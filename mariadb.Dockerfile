FROM rust:latest AS build

RUN apt update && apt upgrade -y && apt install -y git clang-19 lld-19 llvm-19 && \
    rustup default stable

COPY . /work
WORKDIR /work

ARG FLAGS="-C debuginfo=0 -C codegen-units=1 -C opt-level=3 -C linker-plugin-lto -C lto=thin -C embed-bitcode"

RUN RUSTFLAGS="-C linker=clang-19 -C link-arg=-fuse-ld=lld $FLAGS" cargo build --release -p daemon_mariadb_plugin

RUN /usr/lib/llvm-19/bin/llvm-strip target/release/libdaemon_mariadb_plugin.so

FROM mariadb:latest

COPY --from=build /work/target/release/libdaemon_mariadb_plugin.so /usr/lib/mysql/plugin/libdaemon_mariadb_plugin.so

