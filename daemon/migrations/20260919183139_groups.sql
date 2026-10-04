-- Add migration script here

CREATE TABLE `groups` (
	`name` VARCHAR(200) NOT NULL PRIMARY KEY,
	`cpu` FLOAT(255,2) NULL,
    `ram` INT UNSIGNED NULL,
    `swap` INT UNSIGNED NULL,
    CONSTRAINT NAME_CHECK CHECK (`name` RLIKE '^[A-Za-z0-9_]+(-[A-Za-z0-9_]+){0,15}$' AND CHAR_LENGTH(`name`) BETWEEN 1 AND 200)
);

ALTER TABLE `servers`
	ADD COLUMN `group` VARCHAR(200) NULL,
	ADD FOREIGN KEY (`group`) REFERENCES `groups`(`name`) ON DELETE SET NULL;
