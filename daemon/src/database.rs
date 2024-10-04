use crate::schema;
use axum::async_trait;
use crate::common::payload::database::DatabasePayload;
use diesel::result::Error;
use diesel::{delete, insert_into};
use diesel::{BoolExpressionMethods, ExpressionMethods};
use diesel_async::{AsyncMysqlConnection, RunQueryDsl, SimpleAsyncConnection};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use tokio::task::JoinError;

#[allow(unused)]
#[derive(Debug)]
pub(crate) enum DatabaseUpdateError {
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

#[async_trait]
pub(crate) trait DatabasePayloadFunctionExtension {
    async fn update_validated(
        &mut self,
        id: u32,
        conn: &mut AsyncMysqlConnection,
        to: Self,
    ) -> Result<(), DatabaseUpdateError>;

    async fn update_unvalidated(
        &mut self,
        id: u32,
        conn: &mut AsyncMysqlConnection,
        to: Self,
    ) -> Result<(), DatabaseUpdateError>;

    fn validate_hash(sha256: bool, str: &str) -> Result<(), DatabaseUpdateError>
    where
        Self: Sized;

    fn validate_sql_string(str: &str) -> Result<(), DatabaseUpdateError>
    where
        Self: Sized;

    fn permissions(flag: i32) -> HashSet<&'static str>
    where
        Self: Sized;
}

#[async_trait]
impl DatabasePayloadFunctionExtension for DatabasePayload {
    async fn update_validated(
        &mut self,
        id: u32,
        conn: &mut AsyncMysqlConnection,
        to: DatabasePayload,
    ) -> Result<(), DatabaseUpdateError> {
        for (user, data) in &to.users {
            Self::validate_sql_string(user)?;
            Self::validate_hash(data.restricted.is_some(), &data.password_hash)?;
        }
        self.update_unvalidated(id, conn, to).await
    }

    async fn update_unvalidated(
        &mut self,
        id: u32,
        conn: &mut AsyncMysqlConnection,
        to: DatabasePayload,
    ) -> Result<(), DatabaseUpdateError> {
        let name = self.name_suffix.clone();

        if !self.name_suffix.eq(&to.name_suffix) {
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

        let mut sql = String::new();

        for user in remove_users {
            sql += &format!(
                "DROP USER IF EXISTS '{}{}_{}'@'%';",
                crate::common::MARIADB_USER_PREFIX,
                id,
                user
            );
            delete(schema::database_user::dsl::database_user)
                .filter(
                    schema::database_user::dsl::database_id
                        .eq(id)
                        .and(schema::database_user::dsl::name_suffix.eq(&user)),
                )
                .execute(conn)
                .await?;
        }
        for server in remove_server {
            sql += &format!(
                "REVOKE ALL PRIVILEGES ON {}{}_{}.* FROM 'server_{}'@'%';",
                crate::common::MARIADB_DATABASE_PREFIX,
                id,
                name,
                server
            );
            delete(schema::database_passwordless::dsl::database_passwordless)
                .filter(
                    schema::database_passwordless::dsl::database_id
                        .eq(id)
                        .and(schema::database_passwordless::dsl::server_id.eq(server)),
                )
                .execute(conn)
                .await?;
        }
        for (user, data) in add_users {
            match data.restricted {
                None => {
                    sql += &format!(
                                "CREATE USER '{}{}_{}'@'%' IDENTIFIED WITH 'mysql_native_password' AS '{}';",
                                crate::common::MARIADB_USER_PREFIX,
                                id,
                                user,
                                data.password_hash
                            );
                }
                Some(ref servers) => {
                    let mut extra = data.password_hash.clone();
                    for server in servers {
                        extra += &format!("{}\\0", server);
                    }
                    extra += "\\0";
                    sql += &format!(
                        "CREATE USER '{}{}_{}'@'%' IDENTIFIED WITH '{}' AS '{}';",
                        crate::common::MARIADB_USER_PREFIX,
                        id,
                        user,
                        crate::common::MARIADB_PLUGIN_NAME,
                        extra
                    );
                }
            }
            for perm in Self::permissions(data.permissions) {
                sql += &format!(
                    "GRANT {} ON TABLE {}{}_{}.* TO '{}{}_{}'@'%';",
                    perm,
                    crate::common::MARIADB_DATABASE_PREFIX,
                    id,
                    name,
                    crate::common::MARIADB_USER_PREFIX,
                    id,
                    user
                );
            }
            let restricted = data.restricted.as_ref().map(|servers| {
                servers
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(";")
            });
            insert_into(schema::database_user::dsl::database_user)
                .values((
                    schema::database_user::dsl::database_id.eq(id),
                    schema::database_user::dsl::name_suffix.eq(&user),
                    schema::database_user::dsl::password_hash.eq(data.password_hash),
                    schema::database_user::dsl::permissions.eq(data.permissions),
                    schema::database_user::dsl::restricted.eq(restricted),
                ))
                .execute(conn)
                .await?;
        }
        for (server, permissions) in add_server {
            sql += &format!(
                "CREATE USER IF NOT EXISTS 'server_{}'@'%' IDENTIFIED WITH mysql_no_login;",
                server
            );
            sql += &format!("GRANT PROXY ON 'server_{}'@'%' TO 'server'@'%';", server);
            for perm in Self::permissions(permissions) {
                sql += &format!(
                    "GRANT {} ON TABLE {}{}_{}.* TO 'server_{}'@'%';",
                    perm,
                    crate::common::MARIADB_DATABASE_PREFIX,
                    id,
                    name,
                    server
                );
            }
            insert_into(schema::database_passwordless::dsl::database_passwordless)
                .values((
                    schema::database_passwordless::dsl::database_id.eq(id),
                    schema::database_passwordless::dsl::permissions.eq(permissions),
                ))
                .execute(conn)
                .await?;
        }
        sql += "FLUSH PRIVILEGES;";
        conn.batch_execute(&sql).await?;
        Ok(())
    }

    fn validate_hash(sha256: bool, str: &str) -> Result<(), DatabaseUpdateError> {
        if sha256 {
            if !str.len() == crate::common::MARIADB_AUTH_HASH_HEX_SIZE {
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
