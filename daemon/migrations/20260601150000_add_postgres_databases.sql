ALTER TABLE `databases`
  MODIFY COLUMN `type` ENUM('mysql', 'redis', 'postgres') NOT NULL DEFAULT 'mysql';
