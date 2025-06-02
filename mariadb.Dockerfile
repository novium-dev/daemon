FROM rust:alpine AS build

RUN apk add --no-cache gcompat git llvm20 clang20 lld20 --repository=https://dl-cdn.alpinelinux.org/alpine/edge/main && \
    rustup target add x86_64-unknown-linux-gnu

COPY . /work
WORKDIR /work

ARG FLAGS="-C debuginfo=0 -C codegen-units=1 -C opt-level=3 -C linker-plugin-lto -C lto=thin -C embed-bitcode -C link-arg=-Wl,--icf=safe"

RUN CC="clang" CXX="clang++" LD="ld.lld" RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld $FLAGS" cargo build --target x86_64-unknown-linux-gnu --release -p daemon_mariadb_plugin

RUN llvm-strip target/x86_64-unknown-linux-gnu/release/libdaemon_mariadb_plugin.so

FROM mariadb:latest

RUN echo "[mariadb]" > /etc/mysql/mariadb.conf.d/daemon.cnf && \
    echo "plugin_load_add = novium_daemon_auth=libdaemon_mariadb_plugin.so" >> /etc/mysql/mariadb.conf.d/daemon.cnf

COPY --from=build /work/target/x86_64-unknown-linux-gnu/release/libdaemon_mariadb_plugin.so /usr/lib/mysql/plugin/libdaemon_mariadb_plugin.so

