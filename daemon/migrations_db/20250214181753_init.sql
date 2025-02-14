CREATE TABLE IF NOT EXISTS `databases`
(
  `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
  `name_suffix` VARCHAR(255) NOT NULL
);

CREATE TABLE IF NOT EXISTS `database_passwordless`
(
  `database` INT(255) UNSIGNED NOT NULL,
  `server` INT(255) UNSIGNED NOT NULL,
  `permissions` INT(255) NOT NULL,
  PRIMARY KEY (`database`, `server`)
);

CREATE TABLE IF NOT EXISTS `database_user`
(
  `database` INT(255) UNSIGNED NOT NULL,
  `name_suffix` VARCHAR(255) NOT NULL,
  `password_hash` VARCHAR(255) NOT NULL,
  `permissions` INT(255) NOT NULL,
  PRIMARY KEY (`database`, `name_suffix`)
);

CREATE USER IF NOT EXISTS 'server'@'%' IDENTIFIED WITH 'novium_daemon_auth';
