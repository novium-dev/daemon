use crate::common::payload::database::DatabasePayload;
use crate::common::payload::IdResponse;
use crate::db::database::{DatabasePayloadFunctionExtension, DatabaseUpdateError};
use crate::utils::database;
use crate::web::diesel_error_to_http;
use crate::web::payload::FromFetchedDatabaseExtension;
use crate::{db::schema, sql, DaemonState};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use diesel::{delete, insert_into, select, ExpressionMethods};
use diesel_async::{AsyncConnection, AsyncMysqlConnection, RunQueryDsl};
use std::collections::HashMap;

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
