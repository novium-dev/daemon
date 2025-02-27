ALTER TABLE `database_passwordless` ADD FOREIGN KEY (`database`) REFERENCES `databases`(`id`);
ALTER TABLE `database_user` ADD FOREIGN KEY (`database`) REFERENCES `databases`(`id`);
