use crate::database::{Database, DatabasePayloadFunctionExt as _, DatabaseState, FetchedDatabase};
use crate::utils::database;
use crate::{Daemon, sql};
use async_trait::async_trait;
use daemon_common::service::database::DatabaseService;
use daemon_common::service::database::error::{
    CreateDatabaseError, DeleteDatabaseError, UpdateDatabaseError,
};
use daemon_common::service::database::{DatabaseUserPayload, DatabaseUserPermissions};
use sqlx::{Connection as _, query};
use std::collections::HashMap;
use tracing::{debug, error};

#[async_trait]
impl DatabaseService for Daemon {
    async fn create_database(&self, name_suffix: String) -> Result<u32, CreateDatabaseError> {
        Database::validate_sql_string::<CreateDatabaseError>(&name_suffix)
            .inspect_err(|_err| debug!("Validation of name suffix failed: {:?}", name_suffix))?;

        let mut conn1 = self.conn.acquire().await?;
        let mut conn2 = self.conn.acquire().await?;
        tokio::task::spawn(async move {
            conn1
                .transaction(move |tx| {
                    Box::pin(async move {
                        let database_id = query!(
                            "INSERT INTO `databases` (`name_suffix`) VALUES (?)",
                            name_suffix
                        )
                        .execute(&mut **tx)
                        .await?
                        .last_insert_id() as u32;

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

                        Ok::<_, CreateDatabaseError>(database_id)
                    })
                })
                .await
        })
        .await
        .expect("database create task paniced")
    }

    async fn update_database(
        &self,
        database_id: u32,
        users: HashMap<String, DatabaseUserPayload>,
        passwordless: HashMap<u32, DatabaseUserPermissions>,
    ) -> Result<(), UpdateDatabaseError> {
        let Some(database) =
            database::<UpdateDatabaseError>(&*self.conn, database_id).await?
        else {
            return Err(UpdateDatabaseError::DatabaseNotFound);
        };
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
        .expect("update database task paniced")?;
        Ok(())
    }

    async fn delete_database(&self, database_id: u32) -> Result<(), DeleteDatabaseError> {
        let Some(database) =
            database::<DeleteDatabaseError>(&*self.conn, database_id).await?
        else {
            return Err(DeleteDatabaseError::DatabaseNotFound);
        };
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
        .expect("delete database task paniced")?;
        Ok(())
    }
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
