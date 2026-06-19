CREATE TABLE IF NOT EXISTS `images`
(
    `id` BIGINT UNSIGNED NOT NULL PRIMARY KEY,
    `dockerfile` VARCHAR(10000) NOT NULL,
    `default_tag` VARCHAR(255) NULL
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `servers`
(
  `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
  `image` BIGINT UNSIGNED NULL,
  `cpu` FLOAT(255,2) NULL,
  `ram` INT UNSIGNED NULL,
  `swap` INT UNSIGNED NULL,
  `disk` BIGINT UNSIGNED NULL,
  `storage` VARCHAR(255) NOT NULL DEFAULT 'local',
  `internet` BOOLEAN NOT NULL DEFAULT FALSE,
  `ipv6_suffix` BIGINT UNSIGNED DEFAULT NULL,
  `uid` SMALLINT UNSIGNED DEFAULT NULL,
  `gid` SMALLINT UNSIGNED DEFAULT NULL,
  `stop_command` VARCHAR(255) DEFAULT NULL,
  `stop_timeout` SMALLINT UNSIGNED NOT NULL DEFAULT 60,
  FOREIGN KEY (`image`) REFERENCES `images`(`id`),
  CONSTRAINT MAX_DISK CHECK ( (`disk`*1024*1024) <= ((POWER(2, 64))/2)-1 ) -- i64 is used heavy (where -1 means null) in the daemon. Do not allow disk limits that would overflow
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_locks`
(
    `server` INT(255) UNSIGNED NOT NULL,
    `lock` VARCHAR(255) NOT NULL,
    PRIMARY KEY (`server`, `lock`),
    FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_capability_add`
(
  `server` INT UNSIGNED NOT NULL,
  `capability` VARCHAR(255) NOT NULL,
  PRIMARY KEY (`server`, `capability`),
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_capability_drop`
(
  `server` INT(255) UNSIGNED NOT NULL,
  `capability` VARCHAR(255) NOT NULL,
  PRIMARY KEY (`server`, `capability`),
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_environment`
(
  `server` INT(255) UNSIGNED NOT NULL,
  `key` VARCHAR(255) NOT NULL,
  `value` VARCHAR(255) NOT NULL,
  PRIMARY KEY (`server`, `key`),
  CONSTRAINT VALIDATE CHECK (`key` NOT LIKE '%=%'),
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_image_tags`
(
  `server` INT(255) UNSIGNED NOT NULL,
  `image` BIGINT UNSIGNED NOT NULL,
  `tag` VARCHAR(255) NOT NULL,
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE,
  FOREIGN KEY (`image`) REFERENCES `images`(`id`),
  PRIMARY KEY (`server`, `image`)
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_image_args`
(
  `server` INT(255) UNSIGNED NOT NULL,
  `image` BIGINT UNSIGNED NOT NULL,
  `key` VARCHAR(255) NOT NULL,
  `value` VARCHAR(255) NOT NULL,
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE,
  FOREIGN KEY (`image`) REFERENCES `images`(`id`),
  PRIMARY KEY (`server`, `image`, `key`)
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_backup_exclude`
(
  `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
  `server` INT(255) UNSIGNED NOT NULL,
  `exclude` VARCHAR(255) NOT NULL,
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_file_writes`
(
  `server` INT(255) UNSIGNED NOT NULL,
  `file` VARCHAR(255) NOT NULL,
  `filetype` ENUM('write', 'properties', 'ini', 'json', 'yaml', 'toml', 'xml') NOT NULL,
  `key` VARCHAR(255) NOT NULL DEFAULT '',
  `value` TEXT NOT NULL,
  PRIMARY KEY (`server`, `file`, `key`),
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_ports`
(
  `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
  `server` INT(255) UNSIGNED NOT NULL,
  `internal_port` SMALLINT UNSIGNED NOT NULL,
  `external_port` SMALLINT UNSIGNED NOT NULL,
  `external_ip` VARCHAR(255) NOT NULL,
  `protocol` VARCHAR(255) NOT NULL,
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `image_parents`
(
    `image` BIGINT UNSIGNED NOT NULL,
    `parent` BIGINT UNSIGNED NOT NULL,
    PRIMARY KEY (`image`, `parent`),
    FOREIGN KEY (`image`) REFERENCES `images`(`id`),
    FOREIGN KEY (`parent`) REFERENCES `images`(`id`)
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `networks`
(
  `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
  `ipv4` SMALLINT UNSIGNED NOT NULL UNIQUE,
  `ipv6` SMALLINT UNSIGNED NOT NULL UNIQUE,
  CONSTRAINT IP_RANGE CHECK ((`ipv4` & 0b111111) = 0 AND (`ipv4` >> 8) != 255)
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `server_networks`
(
  `server` INT(255) UNSIGNED NOT NULL,
  `network` INT(255) UNSIGNED NOT NULL,
  `ipv4` TINYINT UNSIGNED NOT NULL,
  `ipv6` BIGINT UNSIGNED NOT NULL,
  PRIMARY KEY (`network`, `server`),
  UNIQUE (`network`, `ipv4`),
  UNIQUE (`network`, `ipv6`),
  FOREIGN KEY (`network`) REFERENCES `networks`(`id`) ON DELETE CASCADE,
  FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE,
  CONSTRAINT IP_RANGE CHECK (`ipv4` > 1 AND (`ipv4` | 0b11000000) != 255 AND (`ipv4` & 0b11000000) = 0 AND (`ipv6` & 0xffff) > 1)
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `schedules` (
    `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
    `server` INT(255) UNSIGNED NOT NULL,
    `policy` ENUM('always', 'running', 'stopped') NOT NULL,
    `schedule` VARCHAR(255) NOT NULL,
    `timezone` VARCHAR(255) NOT NULL,
    FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;

CREATE TABLE IF NOT EXISTS `schedule_steps` (
    `schedule` INT(255) UNSIGNED NOT NULL,
    `index` INT UNSIGNED NOT NULL,
    `value` JSON NOT NULL, -- I do not like json in sql, but its the best way here I guess
    FOREIGN KEY (`schedule`) REFERENCES `schedules`(`id`) ON DELETE CASCADE
) ENGINE=INNODB;
