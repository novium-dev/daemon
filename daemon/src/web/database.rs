use crate::common::payload::database::DatabaseModifyPayload;
use crate::common::payload::IdResponse;
use crate::database::{DatabasePayloadFunctionExtension, DatabaseUpdateError};
use crate::sql::MultiTransactionApplyExt;
use crate::utils::database;
use crate::web::payload::FromFetchedDatabaseExtension;
use crate::web::sql_error_to_http;
use crate::{sql, DaemonState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use daemon_common::payload::database::DatabaseCreatePayload;
use sqlx::{query, Connection};
use std::collections::HashMap;
use std::ops::Deref;
use tracing::debug;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

pub(crate) fn config() -> OpenApiRouter<DaemonState> {
    OpenApiRouter::new()
        .routes(routes!(create_database))
        .routes(routes!(modify_database))
        .routes(routes!(delete_database))
}

#[utoipa::path(put,
    path = "/database",
    tag = "database",
    description = "Creates a new database",
    request_body(
        content = DatabaseCreatePayload,
        description = "The database details",
        content_type = "application/json",
        example = json!({
            "name_suffix": "luckperms"
        })
    ),
    responses(
        (status = 200, description = "Database was created successfully", body = IdResponse, content_type = "application/json"),
        (status = 400, description = "Some parts of the payload are not present"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
     ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn create_database(
    State(state): State<DaemonState>,
    Json(payload): Json<DatabaseCreatePayload>,
) -> Result<Json<IdResponse>, StatusCode> {
    DatabaseModifyPayload::validate_sql_string(&payload.name_suffix)
        .inspect_err(|_err| {
            debug!(
                "Validation of name suffix failed: {:?}",
                payload.name_suffix
            )
        })
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let name_suffix = payload.name_suffix.clone();

    let result = {
        let mut conn1 = state
            .conn
            .deref()
            .acquire()
            .await
            .map_err(sql_error_to_http)?;
        let mut conn2 = state
            .conn
            .deref()
            .acquire()
            .await
            .map_err(sql_error_to_http)?;
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
                    .await?
                    .execute(&mut *conn2)
                    .await?;

                    Ok(database_id)
                })
            })
            .await
    };

    match result {
        Err(DatabaseUpdateError::Sql(err)) => Err(sql_error_to_http(err)),
        Err(DatabaseUpdateError::Invalid) => {
            debug!("Invalid database creation: {:?}", result);
            Err(StatusCode::BAD_REQUEST)
        }
        Err(DatabaseUpdateError::Blocking(_)) => {
            debug!("database create task failed: {:?}", result);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Ok(database_id) => Ok(Json(IdResponse { id: database_id })),
    }
}

#[utoipa::path(patch,
    path = "/database/{database_id}",
    tag = "database",
    description = "Updates an existing database",
    request_body(
        content = DatabaseModifyPayload,
        description = "The new database details",
        content_type = "application/json",
        example = json!({
            "passwordless": {},
            "users": {
                "minigame": {
                    "password_hash": "*94BDCEBE19083CE2A1F959FD02F964C7AF4CFC29",
                    "permissions": -1
                }
            }
        })
    ),
    responses(
        (status = 200, description = "Database was updated successfully"),
        (status = 400, description = "Some parts of the payload are not present"),
        (status = 404, description = "The database does not exist"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 501, description = "You tried to rename a database. This hasn't been implemented yet"),
        (status = 401, description = "Your authorization token is invalid")
    ),
    params(
        ("database_id" = u32, description = "The id of the database"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn modify_database(
    State(state): State<DaemonState>,
    Path((database_id,)): Path<(u32,)>,
    Json(payload): Json<DatabaseModifyPayload>,
) -> Result<(), StatusCode> {
    let Some(database) = database(state.conn.deref(), database_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let (name_suffix, mut database) = DatabaseModifyPayload::from_fetched_database(database);

    let result = {
        let mut conn1 = state.conn.acquire().await.map_err(sql_error_to_http)?;
        let conn2 = state.conn.acquire().await.map_err(sql_error_to_http)?;
        database
            .update_validated(database_id, name_suffix, payload)
            .execute(&mut conn1, conn2)
            .await
    };

    match result {
        Err(DatabaseUpdateError::Sql(err)) => Err(sql_error_to_http(err)),
        Err(DatabaseUpdateError::Invalid) => {
            debug!("Invalid database update: {:?}", result);
            Err(StatusCode::BAD_REQUEST)
        }
        Err(DatabaseUpdateError::Blocking(_)) => {
            debug!("database update task failed: {:?}", result);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Ok(_) => Ok(()),
    }
}

#[utoipa::path(delete,
    path = "/database/{database_id}",
    tag = "database",
    description = "Deletes an existing database",
    responses(
        (status = 200, description = "Database was deleted successfully"),
        (status = 500, description = "An unexpected error occurred"),
        (status = 401, description = "Your authorization token is invalid"),
        (status = 404, description = "The database does not exist")
    ),
    params(
        ("database_id" = u32, description = "The id of the database"),
        ("Authorization", Header, description = "The token used for authentication"),
   )
)]
pub(crate) async fn delete_database(
    State(state): State<DaemonState>,
    Path((database_id,)): Path<(u32,)>,
) -> Result<(), StatusCode> {
    let Some(database) = database(state.conn.deref(), database_id)
        .await
        .map_err(sql_error_to_http)?
    else {
        return Err(StatusCode::NOT_FOUND);
    };
    let (name_suffix, mut database) = DatabaseModifyPayload::from_fetched_database(database);

    let result = {
        let mut conn1 = state.conn.acquire().await.map_err(sql_error_to_http)?;
        let conn2 = state.conn.acquire().await.map_err(sql_error_to_http)?;
        let mut transaction = database.update_validated(
            database_id,
            name_suffix.clone(),
            DatabaseModifyPayload {
                passwordless: HashMap::new(),
                users: HashMap::new(),
            },
        );
        if let Ok(transaction) = &mut transaction {
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
        }
        transaction.execute(&mut conn1, conn2).await
    };

    match result {
        Err(DatabaseUpdateError::Sql(err)) => Err(sql_error_to_http(err)),
        Err(DatabaseUpdateError::Invalid) => {
            debug!("Invalid database delete: {:?}", result);
            Err(StatusCode::BAD_REQUEST)
        }
        Err(DatabaseUpdateError::Blocking(_)) => {
            debug!("database delete task failed: {:?}", result);
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
        Ok(_) => Ok(()),
    }
}
