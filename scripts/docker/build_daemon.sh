#!/bin/sh

mkdir ./migrations && cp daemon/server/migrations/*.sql daemon/server/migrations_db/*.sql ./migrations

"$(realpath "$(dirname "$0")")/setup_db.sh"

RUSTFLAGS="-C opt-level=0" cargo build -p daemon_mariadb_plugin --features ci

mkdir -p /usr/lib/mariadb/plugin/
cp target/debug/libdaemon_mariadb_plugin.so /usr/lib/mysql/plugin/libdaemon_mariadb_plugin.so

mariadb -u root -e "INSTALL PLUGIN novium_daemon_auth SONAME 'libdaemon_mariadb_plugin.so'"

rm ./target -rf

cargo sqlx migrate run --database-url "mysql://root@localhost/daemon?socket=/run/mysqld/mysqld.sock"

export DAEMON_DATABASE_URL="mysql://root@localhost/daemon?socket=/run/mysqld/mysqld.sock"

cargo build --release -p daemon --no-default-features --features "$FEATURES"

/usr/lib/llvm-19/bin/llvm-strip target/release/daemon

