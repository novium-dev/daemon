use crate::common::payload::database::DatabasePayload;
use crate::common::payload::IdResponse;
use crate::database::{DatabasePayloadFunctionExtension, DatabaseUpdateError};
use crate::utils::database;
use crate::web::diesel_error_to_http;
use crate::web::payload::FromFetchedDatabaseExtension;
use crate::{schema, sql, DaemonState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use diesel::{delete, insert_into, select, ExpressionMethods};
use diesel_async::{AsyncConnection, AsyncMysqlConnection, RunQueryDsl};
use std::collections::HashMap;
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
        content = DatabasePayload,
        description = "The database details",
        content_type = "application/json",
        example = json!({
            "name_suffix": "luckperms",
            "passwordless": {
                "1": -1
            },
            "users": {
                "minigame": {
                    "password_hash": "*94BDCEBE19083CE2A1F959FD02F964C7AF4CFC29",
                    "permissions": -1
                }
            }
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
    Json(payload): Json<DatabasePayload>,
) -> Result<Json<IdResponse>, StatusCode> {
    let mut conn = state.conn.clone().lock_owned().await;
    if payload.name_suffix.is_none() {
        return Err(StatusCode::BAD_REQUEST);
    }
    DatabasePayload::validate_sql_string(payload.name_suffix.as_ref().unwrap())
        .map_err(|_| StatusCode::BAD_REQUEST)?;

    let (database_id, result) = conn
        .transaction(|conn| {
            Box::pin(async move {
                let database_id =
                    insert_dummy(payload.name_suffix.as_ref().unwrap().clone(), conn).await?;
                sql::create_database(
                    conn,
                    format!(
                        "{}{}_{}",
                        crate::common::MARIADB_DATABASE_PREFIX,
                        database_id,
                        payload.name_suffix.as_ref().unwrap()
                    ),
                )
                .await?;
                let result = DatabasePayload {
                    name_suffix: payload.name_suffix.clone(),
                    users: HashMap::new(),
                    passwordless: HashMap::new(),
                }
                .update_validated(database_id, conn, payload)
                .await;
                if let Err(DatabaseUpdateError::Diesel(err)) = result {
                    return Err(err);
                }
                Ok((database_id, result))
            })
        })
        .await
        .unwrap_or_else(|err| (0, Err(DatabaseUpdateError::Diesel(err))));

    match result {
        Err(DatabaseUpdateError::Diesel(err)) => Err(diesel_error_to_http(err)),
        Err(DatabaseUpdateError::Invalid) => Err(StatusCode::BAD_REQUEST),
        Err(DatabaseUpdateError::Blocking(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        Err(DatabaseUpdateError::RenameingNotImplemented) => Err(StatusCode::NOT_IMPLEMENTED),
        Ok(_) => Ok(Json(IdResponse { id: database_id })),
    }
}

// ONLY EXECUTE INSIDE A TRANSACTION
pub(crate) async fn insert_dummy(
    name_suffix: String,
    conn: &mut AsyncMysqlConnection,
) -> Result<u32, diesel::result::Error> {
    insert_into(schema::databases::dsl::databases)
        .values((schema::databases::dsl::name_suffix.eq(&name_suffix),))
        .execute(conn)
        .await?;
    Ok(select(schema::last_insert_id())
        .get_result::<u64>(conn)
        .await? as u32)
}

#[utoipa::path(patch,
    path = "/database/{database_id}",
    tag = "database",
    description = "Updates an existing database",
    request_body(
        content = DatabasePayload,
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
    Json(payload): Json<DatabasePayload>,
) -> Result<(), StatusCode> {
    let conn = state.conn.clone().lock_owned().await;
    let (database, mut conn) = database(conn, database_id)
        .await
        .map_err(diesel_error_to_http)?;
    let mut database = DatabasePayload::from_fetched_database(database);

    let result = conn
        .transaction(|conn| {
            Box::pin(async move {
                let result = database.update_validated(database_id, conn, payload).await;
                if let Err(DatabaseUpdateError::Diesel(err)) = result {
                    return Err(err);
                }
                Ok(result)
            })
        })
        .await
        .unwrap_or_else(|err| Err(DatabaseUpdateError::Diesel(err)));

    match result {
        Err(DatabaseUpdateError::Diesel(err)) => Err(diesel_error_to_http(err)),
        Err(DatabaseUpdateError::Invalid) => Err(StatusCode::BAD_REQUEST),
        Err(DatabaseUpdateError::Blocking(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        Err(DatabaseUpdateError::RenameingNotImplemented) => Err(StatusCode::NOT_IMPLEMENTED),
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
    let conn = state.conn.clone().lock_owned().await;
    let (database, mut conn) = database(conn, database_id)
        .await
        .map_err(diesel_error_to_http)?;
    let mut database = DatabasePayload::from_fetched_database(database);

    let result = conn
        .transaction(|conn| {
            Box::pin(async {
                let result = database
                    .update_validated(
                        database_id,
                        conn,
                        DatabasePayload {
                            passwordless: HashMap::new(),
                            users: HashMap::new(),
                            name_suffix: None,
                        },
                    )
                    .await;
                if let Err(DatabaseUpdateError::Diesel(err)) = result {
                    return Err(err);
                }
                sql::drop_database(
                    conn,
                    format!(
                        "{}{}_{}",
                        crate::common::MARIADB_DATABASE_PREFIX,
                        database_id,
                        database.name_suffix.unwrap()
                    ),
                )
                .await?;
                delete(schema::databases::dsl::databases)
                    .filter(schema::databases::dsl::id.eq(database_id))
                    .execute(conn)
                    .await?;
                Ok(result)
            })
        })
        .await
        .unwrap_or_else(|err| Err(DatabaseUpdateError::Diesel(err)));

    match result {
        Err(DatabaseUpdateError::Diesel(err)) => Err(diesel_error_to_http(err)),
        Err(DatabaseUpdateError::Invalid) => Err(StatusCode::BAD_REQUEST),
        Err(DatabaseUpdateError::Blocking(_)) => Err(StatusCode::INTERNAL_SERVER_ERROR),
        Err(DatabaseUpdateError::RenameingNotImplemented) => Err(StatusCode::NOT_IMPLEMENTED),
        Ok(_) => Ok(()),
    }
}
