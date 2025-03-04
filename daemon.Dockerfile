FROM rust:latest AS build

RUN apt update && apt upgrade -y && apt install -y curl mariadb-server mariadb-client git clang-19 lld-19 llvm-19 libbtrfs-dev && \
    rustup default beta

COPY . /work
WORKDIR /work

ARG FLAGS="-C debuginfo=0 -C codegen-units=1 -C opt-level=3 -C linker-plugin-lto -C lto=thin -C embed-bitcode"
ARG FEATURES="default"

RUN RUSTFLAGS="-C opt-level=0" cargo install sqlx-cli --debug --no-default-features --features mysql,rustls

# build
RUN FEATURES="$FEATURES" RUSTFLAGS="-C linker=clang-19 -C link-arg=-fuse-ld=lld $FLAGS" ./scripts/docker/build_daemon.sh

FROM debian:latest

RUN mkdir -p /var/lib/novium_daemon && chown 25335:25335 /var/lib/novium_daemon

WORKDIR /var/lib/novium_daemon

USER 25335:25335

# Copy the binary directly to root directory (/)
COPY --from=build /work/target/release/daemon /daemon

ENTRYPOINT ["/daemon"]
