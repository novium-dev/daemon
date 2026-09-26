-- Add migration script here

CREATE TABLE `groups` (
	`name` VARCHAR(255) NOT NULL PRIMARY KEY,
	`cpu` FLOAT(255,2) NULL,
    `ram` INT UNSIGNED NULL,
    `swap` INT UNSIGNED NULL,
    CONSTRAINT NAME_CHECK CHECK (`name` RLIKE '^[A-Za-z0-9_-]+$')
);

ALTER TABLE `servers`
	ADD COLUMN `group` VARCHAR(255) NULL,
	ADD FOREIGN KEY (`group`) REFERENCES `groups`(`name`) ON DELETE SET NULL;
