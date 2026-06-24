FROM rust:latest AS build

RUN apt update \
	&& apt upgrade -y \
	&& apt install -y lsb-release wget gnupg \
	&& wget https://apt.llvm.org/llvm.sh \
	&& chmod +x llvm.sh \
	&& ./llvm.sh 21

COPY . /work
WORKDIR /work

ARG FLAGS="-C debuginfo=0 -C codegen-units=1 -C opt-level=3 -C link-arg=-Wl,--icf=safe"

RUN CC="clang-21" CXX="clang++-21" LD="ld.lld-21" RUSTFLAGS="-C linker=clang-21 -C link-arg=-fuse-ld=lld $FLAGS" cargo build --target x86_64-unknown-linux-gnu --release -p daemon_mariadb_plugin

FROM mariadb:latest

RUN echo "[mariadb]" > /etc/mysql/mariadb.conf.d/daemon.cnf && \
    echo "plugin_load_add = novium_daemon_auth=libdaemon_mariadb_plugin.so" >> /etc/mysql/mariadb.conf.d/daemon.cnf

COPY --from=build /work/target/x86_64-unknown-linux-gnu/release/libdaemon_mariadb_plugin.so /usr/lib/mysql/plugin/libdaemon_mariadb_plugin.so
