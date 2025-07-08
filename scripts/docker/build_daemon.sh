#!/bin/sh

mkdir ./migrations && cp daemon/migrations/*.sql ./migrations

"$(realpath "$(dirname "$0")")/setup_db.sh"

cargo sqlx database create --database-url "mysql://root@localhost/daemon?socket=/run/mysqld/mysqld.sock"
cargo sqlx migrate run --database-url "mysql://root@localhost/daemon?socket=/run/mysqld/mysqld.sock"

export DAEMON_DATABASE_URL="mysql://root@localhost/daemon?socket=/run/mysqld/mysqld.sock"

echo '#!/bin/ash' > /llvm-config-static.sh && echo 'if [ "$1" = "--libs" ]; then echo "$(/usr/bin/llvm-config "$@" "--link-static") -lzstd"; else /usr/bin/llvm-config "$@"; fi' >> /llvm-config-static.sh && chmod +x /llvm-config-static.sh

LLVM_CONFIG_PATH="/llvm-config-static.sh" cargo build --target x86_64-unknown-linux-musl --release -p daemon --no-default-features --features "$FEATURES"

llvm-strip target/x86_64-unknown-linux-musl/release/daemon
