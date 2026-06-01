// SPDX-License-Identifier: AGPL-3.0-only
use crate::database::{Database, DatabasePayloadFunctionExt as _, DatabaseState, FetchedDatabase};
use crate::server::FetchedServer;
use crate::utils::database;
use crate::{Daemon, DaemonExtra, sql};
use async_trait::async_trait;
use bollard::query_parameters::RemoveContainerOptions;
use daemon_common::service::database::DatabaseService;
use daemon_common::service::database::database_service_error::{
    CreateDatabaseError, DeleteDatabaseError, UpdateDatabaseError,
};
use daemon_common::service::database::{
    DatabaseType, DatabaseUserPayload, DatabaseUserPermissions,
};
use sqlx::{Connection as _, PgConnection, query};
use std::collections::HashMap;
use tracing::{debug, error};

#[async_trait]
impl<T: DaemonExtra> DatabaseService for Daemon<T> {
    async fn create_database(
        &self,
        name_suffix: String,
        database_type: Option<DatabaseType>,
        redis_database_limit: Option<u16>,
    ) -> Result<u32, CreateDatabaseError> {
        Database::validate_sql_string::<CreateDatabaseError>(&name_suffix)
            .inspect_err(|_err| debug!("Validation of name suffix failed: {:?}", name_suffix))?;

        let database_type = database_type.unwrap_or_default();
        let database_type_sql = match database_type {
            DatabaseType::Mysql => "mysql",
            DatabaseType::Redis => "redis",
            DatabaseType::Postgres => "postgres",
        };
        let redis_database_limit = redis_database_limit.map(u32::from);
        let postgres_url = if database_type == DatabaseType::Postgres {
            Some(
                self.config
                    .postgres
                    .as_ref()
                    .ok_or(CreateDatabaseError::DatabaseNotConfigured)?
                    .database
                    .clone(),
            )
        } else {
            None
        };

        let mut conn1 = self.conn.acquire().await?;
        let mut conn2 = self.conn.acquire().await?;
        tokio::task::spawn(async move {
            conn1
                .transaction(move |tx| {
                    Box::pin(async move {
                        let database_id = query!(
                            "INSERT INTO `databases` (`name_suffix`, `type`, `redis_database_limit`) VALUES (?, ?, ?)",
                            name_suffix,
                            database_type_sql,
                            redis_database_limit
                        )
                        .execute(&mut **tx)
                        .await?
                        .last_insert_id() as u32;

                        if database_type == DatabaseType::Mysql {
                            sql::create_database(format!(
                                "{}{}_{}",
                                crate::common::MARIADB_DATABASE_PREFIX,
                                database_id,
                                name_suffix
                            ))
                            .prepare(&mut *conn2)
                            .await
                            .inspect_err(|err| {
                                error!("Unable to prepare database creation statement: {:?}", err);
                            })?
                            .execute(&mut *conn2)
                            .await
                            .inspect_err(|err| {
                                error!("Unable to execute database creation statement: {:?}", err);
                            })?;
                        }
                        if database_type == DatabaseType::Postgres {
                            let mut postgres = PgConnection::connect(
                                postgres_url
                                    .as_deref()
                                    .expect("postgres url checked before task spawn"),
                            )
                            .await?;
                            sqlx::query(&format!(
                                "CREATE DATABASE {}",
                                postgres_database_name(database_id, &name_suffix)
                            ))
                            .execute(&mut postgres)
                            .await?;
                        }

                        Ok::<_, CreateDatabaseError>(database_id)
                    })
                })
                .await
        })
        .await
        .expect("database create task panicked")
    }

    async fn update_database(
        &self,
        database_id: u32,
        users: HashMap<String, DatabaseUserPayload>,
        passwordless: HashMap<u32, DatabaseUserPermissions>,
    ) -> Result<(), UpdateDatabaseError> {
        let Some(database) = database::<UpdateDatabaseError>(&*self.conn, database_id).await?
        else {
            return Err(UpdateDatabaseError::DatabaseNotFound);
        };
        if database.database_type == DatabaseType::Redis && !users.is_empty() {
            return Err(UpdateDatabaseError::DatabaseUnsupportedAuth);
        }
        if database.database_type == DatabaseType::Postgres {
            return Err(UpdateDatabaseError::DatabaseUnsupportedAuth);
        }
        if database.database_type == DatabaseType::Redis {
            for permissions in passwordless.values() {
                if permissions.is_empty() {
                    return Err(UpdateDatabaseError::DatabasePermissionlessEntry);
                }
            }

            let mut conn = self.conn.acquire().await?;
            conn.transaction(|tx| {
                Box::pin(async move {
                    query!(
                        "DELETE FROM `database_passwordless` WHERE `database` = ?",
                        database_id
                    )
                    .execute(&mut **tx)
                    .await?;

                    for (server, permissions) in passwordless {
                        query!(
                            "INSERT INTO `database_passwordless` (`database`, `server`, `permissions`) VALUES (?, ?, ?)",
                            database_id,
                            server,
                            permissions.bits()
                        )
                        .execute(&mut **tx)
                        .await?;
                    }

                    Ok::<_, UpdateDatabaseError>(())
                })
            })
            .await?;

            return Ok(());
        }

        let payload = DatabaseState {
            users,
            passwordless,
        };
        let (name_suffix, mut database) = DatabaseState::from_fetched_database(database);

        let mut conn1 = self.conn.acquire().await?;
        let conn2 = self.conn.acquire().await?;
        tokio::task::spawn(async move {
            database
                .update_validated::<UpdateDatabaseError>(database_id, name_suffix, payload)?
                .execute(&mut conn1, conn2)
                .await
                .inspect_err(|err| error!("Unable to execute database update: {:?}", err))?;
            Ok::<_, UpdateDatabaseError>(())
        })
        .await
        .expect("update database task panicked")?;
        Ok(())
    }

    async fn delete_database(&self, database_id: u32) -> Result<(), DeleteDatabaseError> {
        let Some(database) = database::<DeleteDatabaseError>(&*self.conn, database_id).await?
        else {
            return Err(DeleteDatabaseError::DatabaseNotFound);
        };
        if database.database_type == DatabaseType::Redis {
            let server_ids = query!(
                "SELECT `server` FROM `database_passwordless` WHERE `database` = ?",
                database_id
            )
            .fetch_all(&*self.conn)
            .await?;
            for row in server_ids {
                let _ = self
                    .docker
                    .remove_container(
                        &FetchedServer::valkey_container_name(row.server, database_id),
                        Some(RemoveContainerOptions {
                            force: true,
                            ..RemoveContainerOptions::default()
                        }),
                    )
                    .await;
            }

            let mut conn = self.conn.acquire().await?;
            conn.transaction(|tx| {
                Box::pin(async move {
                    query!(
                        "DELETE FROM `database_passwordless` WHERE `database` = ?",
                        database_id
                    )
                    .execute(&mut **tx)
                    .await?;
                    query!(
                        "DELETE FROM `database_user` WHERE `database` = ?",
                        database_id
                    )
                    .execute(&mut **tx)
                    .await?;
                    query!("DELETE FROM `databases` WHERE `id` = ?", database_id)
                        .execute(&mut **tx)
                        .await?;

                    Ok::<_, DeleteDatabaseError>(())
                })
            })
            .await?;

            return Ok(());
        }
        if database.database_type == DatabaseType::Postgres {
            let postgres_url = self
                .config
                .postgres
                .as_ref()
                .ok_or(DeleteDatabaseError::DatabaseNotConfigured)?
                .database
                .clone();
            let mut postgres = PgConnection::connect(&postgres_url).await?;
            sqlx::query(&format!(
                "DROP DATABASE IF EXISTS {}",
                postgres_database_name(database_id, &database.name_suffix)
            ))
            .execute(&mut postgres)
            .await?;

            let mut conn = self.conn.acquire().await?;
            conn.transaction(|tx| {
                Box::pin(async move {
                    query!(
                        "DELETE FROM `database_passwordless` WHERE `database` = ?",
                        database_id
                    )
                    .execute(&mut **tx)
                    .await?;
                    query!(
                        "DELETE FROM `database_user` WHERE `database` = ?",
                        database_id
                    )
                    .execute(&mut **tx)
                    .await?;
                    query!("DELETE FROM `databases` WHERE `id` = ?", database_id)
                        .execute(&mut **tx)
                        .await?;

                    Ok::<_, DeleteDatabaseError>(())
                })
            })
            .await?;

            return Ok(());
        }

        let (name_suffix, mut database) = DatabaseState::from_fetched_database(database);

        let mut conn1 = self.conn.acquire().await?;
        let conn2 = self.conn.acquire().await?;
        tokio::task::spawn(async move {
            let mut transaction = database.update_unvalidated(
                database_id,
                name_suffix.clone(),
                DatabaseState {
                    passwordless: HashMap::new(),
                    users: HashMap::new(),
                },
            );
            transaction.add(
                sql::drop_database(format!(
                    "{}{}_{}",
                    crate::common::MARIADB_DATABASE_PREFIX,
                    database_id,
                    name_suffix
                )),
                vec![query!(
                    "DELETE FROM `databases` WHERE `id` = ?",
                    database_id
                )],
            );
            transaction
                .execute(&mut conn1, conn2)
                .await
                .inspect_err(|err| {
                    error!("Unable to execute database delete statement: {:?}", err);
                })?;
            Ok::<_, DeleteDatabaseError>(())
        })
        .await
        .expect("delete database task panicked")?;
        Ok(())
    }
}

fn postgres_database_name(database_id: u32, name_suffix: &str) -> String {
    format!(
        "{}{}_{}",
        crate::common::POSTGRES_DATABASE_PREFIX,
        database_id,
        name_suffix
    )
}

pub trait FromFetchedDatabaseExtension {
    fn from_fetched_database(value: FetchedDatabase) -> (String, Self);
}

impl FromFetchedDatabaseExtension for DatabaseState {
    fn from_fetched_database(value: FetchedDatabase) -> (String, Self) {
        (
            value.name_suffix,
            Self {
                passwordless: value
                    .passwordless
                    .into_iter()
                    .map(|(key, value)| (key, DatabaseUserPermissions::from_bits_truncate(value)))
                    .collect(),
                users: value
                    .users
                    .into_iter()
                    .map(|(name_suffix, user)| {
                        (
                            name_suffix,
                            DatabaseUserPayload {
                                permissions: DatabaseUserPermissions::from_bits_truncate(
                                    user.permissions,
                                ),
                                password_hash: user.password_hash,
                            },
                        )
                    })
                    .collect(),
            },
        )
    }
}
