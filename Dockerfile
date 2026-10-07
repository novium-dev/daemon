# syntax=docker/dockerfile:1
FROM lukemathwalker/cargo-chef:latest-rust-1.98.0-alpine AS chef
WORKDIR /work

# Host key for private git dependencies (fetched via forwarded SSH agent, see --mount=type=ssh)
RUN mkdir -p /root/.ssh && echo "git-ssh.novium.world ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBPS0cpItg26j3t/ZqMNl+7PY2GwdqVEx4r4pNJMfwf7NvccvVJubH9bD8DHtqVXsBGLtF38SDVb7ycE+7j0TgdE=" > /root/.ssh/known_hosts

ARG FLAGS="-C debuginfo=0 -C codegen-units=1 -C opt-level=3 -C link-arg=-Wl,--icf=safe -C linker-plugin-lto -C lto=thin -C embed-bitcode"
ENV RUSTFLAGS="-C linker=clang-22 -C link-arg=-fuse-ld=lld $FLAGS"

FROM chef AS utils

RUN RUSTFLAGS="-C opt-level=0" cargo install sqlx-cli --no-default-features --features mysql,rustls --locked

FROM chef AS dependencies

RUN apk add --no-cache ncurses-dev ncurses-static zlib-dev zlib-static zstd-dev zstd-static llvm22-static llvm22-dev git mariadb mariadb-client musl-dev llvm22 clang22 clang22-static lld22 curl btrfs-progs-dev --repository=https://dl-cdn.alpinelinux.org/alpine/edge/main

FROM dependencies AS mariadb

RUN mariadb-install-db --user=root --ldata=/var/lib/mysql
RUN echo "USE mysql;FLUSH PRIVILEGES ;GRANT ALL ON *.* TO 'root'@'%' identified by '' WITH GRANT OPTION ;GRANT ALL ON *.* TO 'root'@'localhost' identified by '' WITH GRANT OPTION ;SET PASSWORD FOR 'root'@'localhost'=PASSWORD('') ;DROP DATABASE IF EXISTS test ;FLUSH PRIVILEGES ;" mariadbd --user=root --bootstrap --verbose=0 --skip-name-resolve --skip-networking=0

FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM dependencies AS builder

RUN echo '#!/bin/ash' > /llvm-config-static.sh && echo 'if [[ "$1" == "--libs" ]]; then echo "$(/usr/bin/llvm-config-22 "$@" "--link-static")" "-lzstd"; else /usr/bin/llvm-config-22 "$@"; fi' >> /llvm-config-static.sh && chmod +x /llvm-config-static.sh
ENV LLVM_CONFIG_PATH="/llvm-config-static.sh"

COPY --from=planner /work/recipe.json recipe.json

ARG PREPARE_FEATURES="ci"

RUN --mount=type=ssh cargo chef cook --release --recipe-path recipe.json --no-default-features --features "$PREPARE_FEATURES" --bin daemon --target x86_64-unknown-linux-musl

COPY --from=utils /usr/local/cargo/bin/cargo-sqlx /usr/local/cargo/bin/cargo-sqlx
COPY --from=mariadb /var/lib/mysql /var/lib/mysql
COPY . .

ARG FEATURES="ci"

# build
RUN --mount=type=ssh FEATURES="$FEATURES" ./scripts/docker/build_daemon.sh

FROM alpine:latest AS runtime

RUN ln -sf /bin/daemon /usr/bin/daemon

RUN mkdir -p /var/lib/novium_daemon && chown 25335:25335 /var/lib/novium_daemon

WORKDIR /var/lib/novium_daemon

STOPSIGNAL SIGINT

EXPOSE 443

RUN apk add --no-cache vim --repository=https://dl-cdn.alpinelinux.org/alpine/edge/main

# Copy the binary directly to bin directory (/bin)
COPY --from=builder /work/target/x86_64-unknown-linux-musl/release/daemon /bin/daemon

ENTRYPOINT ["/bin/daemon"]
