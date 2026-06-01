ALTER TABLE `server_file_writes`
  MODIFY `filetype` ENUM('write', 'properties', 'ini', 'json', 'yaml') NOT NULL;
