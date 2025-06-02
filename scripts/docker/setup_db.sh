#!/bin/sh

mkdir /run/mysqld -p
mariadb-install-db --user=root --ldata=/var/lib/mysql
echo "USE mysql;FLUSH PRIVILEGES ;GRANT ALL ON *.* TO 'root'@'%' identified by '' WITH GRANT OPTION ;GRANT ALL ON *.* TO 'root'@'localhost' identified by '' WITH GRANT OPTION ;SET PASSWORD FOR 'root'@'localhost'=PASSWORD('') ;DROP DATABASE IF EXISTS test ;FLUSH PRIVILEGES ;" mariadbd --user=root --bootstrap --verbose=0 --skip-name-resolve --skip-networking=0

mariadbd --user=root --skip-name-resolve --skip-networking=0 --silent-startup &

until mariadb-admin ping --user root; do sleep 1; done
