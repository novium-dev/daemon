ALTER TABLE `databases`
  ADD COLUMN `type` ENUM('mysql', 'redis', 'postgres') NOT NULL DEFAULT 'mysql',
  ADD COLUMN `redis_memory_mib` INT UNSIGNED NULL;
