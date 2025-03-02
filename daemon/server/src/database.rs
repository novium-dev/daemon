use crate::common::payload::database::DatabaseModifyPayload;
use crate::sql;
use crate::sql::MultiTransactionApply;
use daemon_common::payload::database::DatabaseUserPermissions;
use regex::Regex;
use sqlx::{Error, Executor, MySql, query, query_as};
use std::collections::{HashMap, HashSet};
use tokio::task::JoinError;

#[derive(Clone, Eq, PartialEq, sqlx::FromRow)]
pub(crate) struct Database {
    pub(crate) id: u32,
    pub(crate) name_suffix: String,
}

#[derive(Clone, Default, Eq, PartialEq)]
pub struct FetchedDatabase {
    pub name_suffix: String,
    pub passwordless: HashMap<u32, i8>,
    pub users: HashMap<String, DatabaseUser>,
}

#[derive(Clone, Eq, PartialEq, sqlx::FromRow)]
pub(crate) struct DatabaseUser {
    pub(crate) database: u32,
    pub(crate) name_suffix: String,
    pub(crate) password_hash: String,
    pub(crate) permissions: i8,
}

impl Database {
    pub(crate) async fn fetched(
        self,
        conn: impl Executor<'_, Database = MySql> + Copy,
    ) -> Result<FetchedDatabase, Error> {
        let passwordless = query!(
            "SELECT `server`, `permissions` FROM `database_passwordless` WHERE `database` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(|passwordless| (passwordless.server, passwordless.permissions))
        .collect();

        let users = query_as!(
            DatabaseUser,
            "SELECT * FROM `database_user` WHERE `database` = ?",
            self.id
        )
        .fetch_all(conn)
        .await?
        .into_iter()
        .map(|user| (user.name_suffix.clone(), user))
        .collect();

        Ok(FetchedDatabase {
            name_suffix: self.name_suffix,
            passwordless,
            users,
        })
    }
}

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum DatabaseUpdateError {
    Sql(sqlx::Error),
    Invalid,
    Blocking(JoinError),
}

impl From<sqlx::Error> for DatabaseUpdateError {
    fn from(value: sqlx::Error) -> Self {
        DatabaseUpdateError::Sql(value)
    }
}

impl From<JoinError> for DatabaseUpdateError {
    fn from(value: JoinError) -> Self {
        DatabaseUpdateError::Blocking(value)
    }
}

pub(crate) trait DatabasePayloadFunctionExtension {
    fn update_validated(
        &mut self,
        id: u32,
        name_suffix: String,
        to: Self,
    ) -> Result<MultiTransactionApply, DatabaseUpdateError>;

    fn update_unvalidated(
        &mut self,
        id: u32,
        name_suffix: String,
        to: Self,
    ) -> Result<MultiTransactionApply, DatabaseUpdateError>;

    fn validate_hash(str: &str) -> Result<(), DatabaseUpdateError>
    where
        Self: Sized;

    fn validate_sql_string(str: &str) -> Result<(), DatabaseUpdateError>
    where
        Self: Sized;
}

impl DatabasePayloadFunctionExtension for DatabaseModifyPayload {
    fn update_validated(
        &mut self,
        id: u32,
        name_suffix: String,
        to: DatabaseModifyPayload,
    ) -> Result<MultiTransactionApply, DatabaseUpdateError> {
        for (user, data) in &to.users {
            Self::validate_sql_string(user)?;
            Self::validate_hash(&data.password_hash)?;
            if data.permissions.is_empty() {
                return Err(DatabaseUpdateError::Invalid);
            }
        }
        for perms in to.passwordless.values() {
            if perms.is_empty() {
                return Err(DatabaseUpdateError::Invalid);
            }
        }
        self.update_unvalidated(id, name_suffix, to)
    }

    fn update_unvalidated(
        &mut self,
        id: u32,
        name: String,
        to: DatabaseModifyPayload,
    ) -> Result<MultiTransactionApply, DatabaseUpdateError> {
        let mut transaction = MultiTransactionApply::default();

        let mut remove_server = HashSet::new();
        let mut add_server = HashMap::new();

        for server in self.passwordless.keys() {
            if !to.passwordless.contains_key(server) {
                remove_server.insert(*server);
            }
        }
        for (server, permissions) in &to.passwordless {
            if let Some(old_permissions) = self.passwordless.get(server) {
                if !old_permissions.eq(permissions) {
                    remove_server.insert(*server);
                    add_server.insert(*server, *permissions);
                }
            } else {
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
            return Ok(transaction);
        }

        for user in remove_users {
            transaction.add(
                sql::drop_user(
                    format!("{}{}_{}", crate::common::MARIADB_USER_PREFIX, id, user),
                    "%",
                ),
                vec![query!(
                    "DELETE FROM `database_user` WHERE `database` = ? AND `name_suffix` = ?",
                    id,
                    &user
                )],
            );
        }
        for server in remove_server {
            transaction.add(
                sql::revoke_all_privileges(
                    format!("{}{}_{}", crate::common::MARIADB_DATABASE_PREFIX, id, name),
                    format!("server_{}", server),
                    "%",
                ),
                vec![query!(
                    "DELETE FROM `database_passwordless` WHERE `database` = ? AND `server` = ?",
                    id,
                    server
                )],
            );
        }
        for (user, data) in add_users {
            transaction.add(sql::create_user(
                format!("{}{}_{}", crate::common::MARIADB_USER_PREFIX, id, user),
                "%",
                "mysql_native_password",
                Some(data.password_hash.clone()),
                false,
            ), vec![
                query!("INSERT INTO `database_user` (`database`, `name_suffix`, `password_hash`, `permissions`) VALUES (?, ?, ?, 0)", id, &user, data.password_hash)
            ]);
            let mut applied = DatabaseUserPermissions::empty();
            for (perm_name, perm) in data.permissions.iter_names() {
                applied |= perm;
                transaction.add(sql::grant_privilege(
                    format!("{}{}_{}", crate::common::MARIADB_DATABASE_PREFIX, id, name),
                    perm_name,
                    format!("{}{}_{}", crate::common::MARIADB_USER_PREFIX, id, user),
                    "%",
                ), vec![
                    query!("UPDATE `database_user` SET `permissions` = ? WHERE `database` = ? AND `name_suffix` = ?", applied.bits(), id, &user)
                ]);
            }
        }
        for (server, permissions) in add_server {
            transaction.add(sql::create_user(
                format!("server_{}", server),
                "%",
                "mysql_native_password",
                None,
                true,
            ), vec![query!(
                "INSERT INTO `database_passwordless` (`database`, `server`, `permissions`) VALUES (?, ?, 0)",
                id, server,
            )]);
            let mut applied = DatabaseUserPermissions::empty();
            for (perm_name, perm) in permissions.iter_names() {
                applied |= perm;
                transaction.add(sql::grant_privilege(
                    format!("{}{}_{}", crate::common::MARIADB_DATABASE_PREFIX, id, name),
                    perm_name,
                    format!("server_{}", server),
                    "%",
                ), vec![query!("UPDATE `database_passwordless` SET `permissions` = ? WHERE `database` = ? AND `server` = ?", applied.bits(), id, server)])
            }
            transaction.add(
                sql::grant_proxy(format!("server_{}", server), "%", "server", "%"),
                Vec::new(),
            );
        }
        transaction.add(sql::flush_privileges(), Vec::new());
        Ok(transaction)
    }

    fn validate_hash(str: &str) -> Result<(), DatabaseUpdateError> {
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
        Ok(())
    }

    fn validate_sql_string(str: &str) -> Result<(), DatabaseUpdateError> {
        if Regex::new("[a-zA-Z0-9_]+").unwrap().is_match(str) && str.len() <= 32 {
            Ok(())
        } else {
            Err(DatabaseUpdateError::Invalid)
        }
    }
}
