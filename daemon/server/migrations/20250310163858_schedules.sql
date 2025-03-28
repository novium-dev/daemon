
CREATE TABLE IF NOT EXISTS `schedules` (
    `id` INT(255) UNSIGNED NOT NULL AUTO_INCREMENT PRIMARY KEY,
    `server` INT(255) UNSIGNED NOT NULL,
    `policy` ENUM('always', 'running', 'stopped') NOT NULL,
    `schedule` VARCHAR(255) NOT NULL,
    `timezone` VARCHAR(255) NOT NULL,
    FOREIGN KEY (`server`) REFERENCES `servers`(`id`) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS `schedule_steps` (
    `schedule` INT(255) UNSIGNED NOT NULL,
    `index` INT UNSIGNED NOT NULL,
    `value` JSON NOT NULL, -- I do not like json in sql, but its the best way here I guess
    FOREIGN KEY (`schedule`) REFERENCES `schedules`(`id`) ON DELETE CASCADE
);

