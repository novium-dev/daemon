ALTER TABLE `database_passwordless` MODIFY COLUMN `permissions` TINYINT NOT NULL;
ALTER TABLE `database_user` MODIFY COLUMN `permissions` TINYINT NOT NULL;
ALTER TABLE `database_passwordless` ADD FOREIGN KEY (`database`) REFERENCES `databases`(`id`);
ALTER TABLE `database_user` ADD FOREIGN KEY (`database`) REFERENCES `databases`(`id`);
