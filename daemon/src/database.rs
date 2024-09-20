#![allow(dead_code)] // TODO: WIP, remove if ready

use diesel::connection::SimpleConnection;
use diesel::result::Error;
use diesel::{Connection, MysqlConnection};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::task::JoinError;

#[derive(Default)]
pub struct FetchedDatabase {
    pub name_suffix: Option<String>,
    pub passwordless: HashMap<u32, i32>,
    pub users: HashMap<String, FetchedDatabaseUser>,
}

#[derive(Eq, PartialEq, Hash)]
pub struct FetchedDatabaseUser {
    pub permissions: i32,
    pub password_hash: String,
    pub restricted: Option<Vec<u32>>,
}

#[allow(unused)]
#[derive(Debug)]
pub enum DatabaseUpdateError {
    Diesel(diesel::result::Error),
    Invalid,
    Blocking(JoinError),
    RenameingNotImplemented,
}

impl From<diesel::result::Error> for DatabaseUpdateError {
    fn from(value: Error) -> Self {
        DatabaseUpdateError::Diesel(value)
    }
}

impl From<JoinError> for DatabaseUpdateError {
    fn from(value: JoinError) -> Self {
        DatabaseUpdateError::Blocking(value)
    }
}

impl FetchedDatabase {
    pub async fn update_validated(
        &mut self,
        id: u32,
        conn: Arc<Mutex<MysqlConnection>>,
        to: FetchedDatabase,
    ) -> Result<(), DatabaseUpdateError> {
        for (user, data) in &to.users {
            Self::validate_sql_string(user)?;
            Self::validate_hash(data.restricted.is_some(), &data.password_hash)?;
        }
        self.update_unvalidated(id, conn, to).await
    }

    pub async fn update_unvalidated(
        &mut self,
        id: u32,
        conn: Arc<Mutex<MysqlConnection>>,
        to: FetchedDatabase,
    ) -> Result<(), DatabaseUpdateError> {
        let name = self.name_suffix.as_ref().unwrap().clone();

        if to.name_suffix.is_some()
            && !self
                .name_suffix
                .as_ref()
                .unwrap()
                .eq(to.name_suffix.as_ref().unwrap())
        {
            return Err(DatabaseUpdateError::RenameingNotImplemented);
        }

        let mut remove_server = HashSet::new();
        let mut add_server = HashMap::new();

        for server in self.passwordless.keys() {
            if !to.passwordless.contains_key(server) {
                remove_server.insert(*server);
            }
        }
        for (server, permissions) in &to.passwordless {
            if !self.passwordless.contains_key(server) {
                add_server.insert(*server, *permissions);
            }
        }

        let mut remove_users = HashSet::new();
        let mut add_users = HashMap::new();

        for user in self.users.keys() {
            if !to.users.contains_key(user) {
                remove_users.insert(user.clone());
            }
        }

        for (user, data) in to.users {
            if let Some(old_data) = self.users.get(&user) {
                if !old_data.eq(&data) {
                    remove_users.insert(user.clone());
                    add_users.insert(user, data);
                }
            } else {
                add_users.insert(user, data);
            }
        }

        if add_server.is_empty()
            && remove_server.is_empty()
            && add_users.is_empty()
            && remove_users.is_empty()
        {
            return Ok(());
        }

        tokio::task::spawn_blocking(move || {
            let mut conn = conn.blocking_lock();
            // Unsure if a transaction is even necessary here, but It probably doesn't hurt
            // stupid, but still the best way
            conn.transaction(move |conn| {
                let mut sql = String::new();

                for user in remove_users {
                    sql += &format!(
                        "DROP USER IF EXISTS '{}{}_{}'@'%';",
                        crate::MARIADB_USER_PREFIX,
                        id,
                        user
                    );
                }
                for server in remove_server {
                    sql += &format!(
                        "REVOKE ALL PRIVILEGES ON {}{}_{}.* FROM 'server_{}'@'%';",
                        crate::MARIADB_DATABASE_PREFIX,
                        id,
                        name,
                        server
                    );
                }
                for (user, data) in add_users {
                    match data.restricted {
                        None => {
                            sql += &format!(
                                "CREATE USER '{}{}_{}'@'%' IDENTIFIED WITH 'mysql_native_password' AS '{}';",
                                crate::MARIADB_USER_PREFIX,
                                id,
                                user,
                                data.password_hash
                            );
                        }
                        Some(servers) => {
                            let mut extra = data.password_hash.clone();
                            for server in &servers {
                                extra += &format!("{}\\0", server);
                            }
                            extra += "\\0";
                            sql += &format!(
                                "CREATE USER '{}{}_{}'@'%' IDENTIFIED WITH '{}' AS '{}';",
                                crate::MARIADB_USER_PREFIX,
                                id,
                                user,
                                crate::MARIADB_PLUGIN_NAME,
                                extra
                            );
                        }
                    }
                    for perm in Self::permissions(data.permissions) {
                        sql += &format!(
                            "GRANT {} ON TABLE {}{}_{}.* TO '{}{}_{}'@'%';",
                            perm,
                            crate::MARIADB_DATABASE_PREFIX,
                            id,
                            name,
                            crate::MARIADB_USER_PREFIX,
                            id,
                            user
                        );
                    }
                }
                for (server, permissions) in add_server {
                    sql += &format!("CREATE USER IF NOT EXISTS 'server_{}'@'%' IDENTIFIED WITH mysql_no_login;", server);
                    sql += &format!("GRANT PROXY ON 'server_{}'@'%' TO 'server'@'%';", server);
                    for perm in Self::permissions(permissions) {
                        sql += &format!(
                            "GRANT {} ON TABLE {}{}_{}.* TO 'server_{}'@'%';",
                            perm,
                            crate::MARIADB_DATABASE_PREFIX,
                            id,
                            name,
                            server
                        );
                    }
                }
                sql += "FLUSH PRIVILEGES;";
                conn.batch_execute(&sql)?;
                Ok(())
            })
        })
        .await?
    }

    fn validate_hash(sha256: bool, str: &str) -> Result<(), DatabaseUpdateError> {
        if sha256 {
            if !str.len() == crate::MARIADB_AUTH_HASH_HEX_SIZE {
                return Err(DatabaseUpdateError::Invalid);
            }
            for c in str[1..].chars() {
                if !"0123456789abcdef".contains(c) {
                    return Err(DatabaseUpdateError::Invalid);
                }
            }
        } else {
            if !str.len() == 41 {
                return Err(DatabaseUpdateError::Invalid);
            }
            if !str.starts_with('*') {
                return Err(DatabaseUpdateError::Invalid);
            }
            for c in str[1..].chars() {
                if !"0123456789ABCDEF".contains(c) {
                    return Err(DatabaseUpdateError::Invalid);
                }
            }
        }
        Ok(())
    }

    fn validate_sql_string(str: &str) -> Result<(), DatabaseUpdateError> {
        if Regex::new("[a-zA-Z0-9_-]+").unwrap().is_match(str) {
            Ok(())
        } else {
            Err(DatabaseUpdateError::Invalid)
        }
    }

    fn permissions(flag: i32) -> HashSet<&'static str> {
        let mut set = HashSet::new();
        if (flag & 1 << 0) != 0 {
            set.insert("SELECT");
        }
        if (flag & 1 << 1) != 0 {
            set.insert("INSERT");
        }
        if (flag & 1 << 2) != 0 {
            set.insert("UPDATE");
        }
        if (flag & 1 << 3) != 0 {
            set.insert("DELETE");
        }
        if (flag & 1 << 4) != 0 {
            set.insert("CREATE");
        }
        if (flag & 1 << 5) != 0 {
            set.insert("DROP");
        }
        set
    }
}
