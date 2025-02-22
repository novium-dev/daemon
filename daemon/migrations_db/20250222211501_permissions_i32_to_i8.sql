ALTER TABLE `database_passwordless` MODIFY COLUMN `permissions` TINYINT NOT NULL;
ALTER TABLE `database_user` MODIFY COLUMN `permissions` TINYINT NOT NULL;
