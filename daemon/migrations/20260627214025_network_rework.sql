ALTER TABLE `networks`
	MODIFY `ipv4` SMALLINT UNSIGNED UNIQUE DEFAULT NULL;

ALTER TABLE `server_networks`
	MODIFY `ipv4` TINYINT UNSIGNED DEFAULT NULL;

ALTER TABLE `networks`
	DROP CONSTRAINT IP_RANGE,
	ADD CONSTRAINT IP_RANGE CHECK (`ipv4` IS NULL OR ((`ipv4` & 0b111111) = 0 AND (`ipv4` >> 8) != 255));

ALTER TABLE `server_networks`
	DROP CONSTRAINT IP_RANGE,
	ADD CONSTRAINT IP_RANGE CHECK ((`ipv4` IS NULL OR (`ipv4` > 1 AND (`ipv4` | 0b11000000) != 255 AND (`ipv4` & 0b11000000) = 0)) AND (`ipv6` & 0xffff) > 1);

CREATE TABLE `wireguard_tunnels`
(
	`id` INT UNSIGNED NOT NULL PRIMARY KEY,
	`ip` BINARY(16) NOT NULL,
	`port` SMALLINT UNSIGNED NOT NULL
);

CREATE TABLE `wireguard_peers`
(
	`tunnel` INT UNSIGNED NOT NULL,
	`endpoint` VARCHAR(255) NULL,
	`preshared_key` BINARY(32) NOT NULL,
	`ip` BINARY(8) NOT NULL,
	UNIQUE (`tunnel`, `endpoint`),
	UNIQUE (`tunnel`, `ip`),
	FOREIGN KEY (`tunnel`) REFERENCES `wireguard_tunnels`(`id`)
);

ALTER TABLE `networks`
	ADD COLUMN `tunnel` INT UNSIGNED NULL AFTER `id`,
	ADD COLUMN `vlan` INT UNSIGNED NULL AFTER `tunnel`,
	ADD CONSTRAINT CONSISTENCY_CHECK CHECK ((`tunnel` IS NULL) = (`vlan` IS NULL)),
	ADD CONSTRAINT VLAN_CHECK CHECK (`vlan` IS NULL OR (`vlan` & 0xFF000000) = 0),
	ADD UNIQUE (`tunnel`, `vlan`),
	ADD FOREIGN KEY (`tunnel`) REFERENCES `wireguard_tunnels`(`id`);
