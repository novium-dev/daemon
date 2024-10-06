use daemon_common::payload::database::DatabasePermissions;
use diesel::sql_types::Text;
use diesel_async::{AsyncMysqlConnection, RunQueryDsl, SimpleAsyncConnection};

pub(crate) async fn init(conn: &mut AsyncMysqlConnection) -> Result<(), diesel::result::Error> {
    conn.batch_execute(include_str!("sql/init.sql")).await?;

    #[cfg(feature = "database")]
    conn.batch_execute(&include_str!("sql/init_database.sql").replace(
        "{{MARIADB_PLUGIN_NAME}}",
        crate::common::MARIADB_PLUGIN_NAME,
    ))
    .await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn create_user(
    conn: &mut AsyncMysqlConnection,
    user: impl Into<String>,
    host: impl Into<String>,
    plugin: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(include_str!("sql/create_user.sql"))
        .bind::<Text, _>(user.into())
        .bind::<Text, _>(host.into())
        .bind::<Text, _>(plugin.into())
        .execute(conn)
        .await?;
    execute_statement(conn).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn create_user_prehashed(
    conn: &mut AsyncMysqlConnection,
    user: impl Into<String>,
    host: impl Into<String>,
    plugin: impl Into<String>,
    hash: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(include_str!("sql/create_user_prehashed.sql"))
        .bind::<Text, _>(user.into())
        .bind::<Text, _>(host.into())
        .bind::<Text, _>(plugin.into())
        .bind::<Text, _>(hash.into())
        .execute(conn)
        .await?;
    execute_statement(conn).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn drop_user(
    conn: &mut AsyncMysqlConnection,
    user: impl Into<String>,
    host: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(include_str!("sql/drop_user.sql"))
        .bind::<Text, _>(user.into())
        .bind::<Text, _>(host.into())
        .execute(conn)
        .await?;
    execute_statement(conn).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn grant_privileges(
    conn: &mut AsyncMysqlConnection,
    table: impl Into<String>,
    privileges: DatabasePermissions,
    user: impl Into<String>,
    host: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    let table = table.into();
    let user = user.into();
    let host = host.into();
    for privilege in privileges.iter_names().map(|(k, _perm)| k) {
        grant_privilege(conn, &table, privilege, &user, &host).await?;
    }
    Ok(())
}

async fn grant_privilege(
    conn: &mut AsyncMysqlConnection,
    table: impl Into<String>,
    privilege: impl Into<String>,
    user: impl Into<String>,
    host: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(include_str!("sql/grant_privilege.sql"))
        .bind::<Text, _>(privilege.into())
        .bind::<Text, _>(table.into())
        .bind::<Text, _>(user.into())
        .bind::<Text, _>(host.into())
        .execute(conn)
        .await?;
    execute_statement(conn).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn revoke_all_privileges(
    conn: &mut AsyncMysqlConnection,
    table: impl Into<String>,
    user: impl Into<String>,
    host: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(include_str!("sql/revoke_all_privileges.sql"))
        .bind::<Text, _>(table.into())
        .bind::<Text, _>(user.into())
        .bind::<Text, _>(host.into())
        .execute(conn)
        .await?;
    execute_statement(conn).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn grant_proxy(
    conn: &mut AsyncMysqlConnection,
    user: impl Into<String>,
    host: impl Into<String>,
    proxy_user: impl Into<String>,
    proxy_host: impl Into<String>,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query(include_str!("sql/grant_proxy.sql"))
        .bind::<Text, _>(user.into())
        .bind::<Text, _>(host.into())
        .bind::<Text, _>(proxy_user.into())
        .bind::<Text, _>(proxy_host.into())
        .execute(conn)
        .await?;
    execute_statement(conn).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) async fn flush_privileges(
    conn: &mut AsyncMysqlConnection,
) -> Result<(), diesel::result::Error> {
    diesel::sql_query("FLUSH PRIVILEGES").execute(conn).await?;
    Ok(())
}

async fn execute_statement(conn: &mut AsyncMysqlConnection) -> Result<(), diesel::result::Error> {
    conn.batch_execute(include_str!("sql/execute_statement.sql"))
        .await
}
