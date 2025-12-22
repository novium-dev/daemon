FROM rust:latest AS build

RUN apt update && apt upgrade -y && apt install -y llvm clang lld

COPY . /work
WORKDIR /work

ARG FLAGS="-C debuginfo=2 -C codegen-units=1 -C opt-level=2"

RUN CC="clang" CXX="clang++" LD="ld.lld" RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld $FLAGS" cargo build --target x86_64-unknown-linux-gnu --release -p daemon_mariadb_plugin

FROM mariadb:latest

RUN echo "[mariadb]" > /etc/mysql/mariadb.conf.d/daemon.cnf && \
    echo "plugin_load_add = novium_daemon_auth=libdaemon_mariadb_plugin.so" >> /etc/mysql/mariadb.conf.d/daemon.cnf

COPY --from=build /work/target/x86_64-unknown-linux-gnu/release/libdaemon_mariadb_plugin.so /usr/lib/mysql/plugin/libdaemon_mariadb_plugin.so
