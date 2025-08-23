FROM rust:alpine AS build

RUN apk add --no-cache ncurses-dev ncurses-static zlib-dev zlib-static zstd-dev zstd-static llvm20-static llvm20-dev git mariadb mariadb-client musl-dev llvm20 clang20 clang20-static lld20 curl btrfs-progs-dev --repository=https://dl-cdn.alpinelinux.org/alpine/edge/main
RUN apk upgrade --no-cache gcc --repository=https://dl-cdn.alpinelinux.org/alpine/edge/main

ARG FEATURES="ci"

ARG FLAGS="-C debuginfo=0 -C codegen-units=1 -C opt-level=3 -C linker-plugin-lto -C lto=thin -C embed-bitcode -C link-arg=-Wl,--icf=safe"

RUN RUSTFLAGS="-C opt-level=0" cargo install sqlx-cli --debug --no-default-features --features mysql,rustls

ENV RUSTFLAGS="-C linker=clang -C link-arg=-fuse-ld=lld -C link-args=-lzstd $FLAGS"

COPY . /work
WORKDIR /work

# build
RUN FEATURES="$FEATURES" ./scripts/docker/build_daemon.sh

FROM alpine:latest

RUN ln -sf /bin/daemon /usr/bin/daemon

RUN mkdir -p /var/lib/novium_daemon && chown 25335:25335 /var/lib/novium_daemon

WORKDIR /var/lib/novium_daemon

STOPSIGNAL SIGINT

EXPOSE 443

RUN apk add --no-cache vim --repository=https://dl-cdn.alpinelinux.org/alpine/edge/main

# Copy the binary directly to bin directory (/bin)
COPY --from=build /work/target/x86_64-unknown-linux-musl/release/daemon /bin/daemon

ENTRYPOINT ["/bin/daemon"]
