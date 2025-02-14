use sqlx::migrate::Migrator;
use sqlx::mysql::MySqlArguments;
use sqlx::pool::PoolConnection;
use sqlx::query::Query;
use sqlx::{query, raw_sql, Connection, Executor, MySql, Pool};
use std::fmt::Debug;

static MIGRATOR: Migrator = sqlx::migrate!();
static DATABASE_MIGRATOR: Migrator = sqlx::migrate!("./migrations_db");

pub(crate) struct PreparedQuery(Query<'static, MySql, MySqlArguments>);

impl PreparedQuery {
    pub(crate) async fn prepare<'a, E: Executor<'a, Database = MySql>>(
        self,
        executor: E,
    ) -> Result<ExecuteQuery, sqlx::Error> {
        self.0.execute(executor).await?;
        Ok(ExecuteQuery)
    }
}

pub(crate) struct ExecuteQuery;

impl ExecuteQuery {
    pub(crate) async fn execute<'a, E: Executor<'a, Database = MySql>>(
        self,
        executor: E,
    ) -> Result<(), sqlx::Error> {
        executor
            .execute(raw_sql(
                "PREPARE stmt FROM @STATEMENT;EXECUTE stmt;DEALLOCATE PREPARE stmt;",
            ))
            .await?;
        Ok(())
    }
}

#[derive(Default)]
pub(crate) struct MultiTransactionApply(
    Vec<(PreparedQuery, Vec<Query<'static, MySql, MySqlArguments>>)>,
);

impl MultiTransactionApply {
    pub(crate) fn add(
        &mut self,
        main: PreparedQuery,
        transaction_query: Vec<Query<'static, MySql, MySqlArguments>>,
    ) {
        self.0.push((main, transaction_query));
    }

    pub(crate) async fn execute(
        self,
        transaction_conn: &mut PoolConnection<MySql>,
        mut raw_conn: PoolConnection<MySql>,
    ) -> Result<(), sqlx::Error> {
        for (control, queries) in self.0 {
            raw_conn = transaction_conn
                .transaction(move |tx| {
                    Box::pin(async move {
                        for query in queries {
                            tx.execute(query).await?;
                        }
                        control
                            .prepare(&mut *raw_conn)
                            .await?
                            .execute(&mut *raw_conn)
                            .await?;
                        Ok::<_, sqlx::Error>(raw_conn)
                    })
                })
                .await?;
        }
        Ok(())
    }
}

pub(crate) trait MultiTransactionApplyExt<E> {
    async fn execute(
        self,
        transaction_conn: &mut PoolConnection<MySql>,
        raw_conn: PoolConnection<MySql>,
    ) -> Result<(), E>;
}

impl<E: From<sqlx::Error> + Debug> MultiTransactionApplyExt<E>
    for Result<MultiTransactionApply, E>
{
    async fn execute(
        self,
        transaction_conn: &mut PoolConnection<MySql>,
        raw_conn: PoolConnection<MySql>,
    ) -> Result<(), E> {
        match self {
            Ok(transaction) => transaction
                .execute(transaction_conn, raw_conn)
                .await
                .map_err(E::from),
            Err(err) => Err(err),
        }
    }
}

pub(crate) async fn init(pool: &Pool<MySql>) -> Result<(), sqlx::Error> {
    MIGRATOR.run(pool).await?;

    DATABASE_MIGRATOR.run(pool).await?;
    Ok(())
}

#[allow(unused)]
pub(crate) fn create_user(
    user: impl Into<String>,
    host: impl Into<String>,
    plugin: impl Into<String>,
    hash: impl Into<Option<String>>,
    lock: bool,
) -> PreparedQuery {
    let hash = hash.into();
    PreparedQuery(if let Some(hash) = hash {
        if lock {
            query!("SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?), ' AS ', QUOTE(?), ' ACCOUNT LOCK')", user.into(), host.into(), plugin.into(), hash)
        } else {
            query!("SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?), ' AS ', QUOTE(?))", user.into(), host.into(), plugin.into(), hash)
        }
    } else if lock {
        query!("SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?), ' ACCOUNT LOCK')", user.into(), host.into(), plugin.into())
    } else {
        query!("SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?))", user.into(), host.into(), plugin.into())
    })
}

#[allow(unused)]
pub(crate) fn drop_user(user: impl Into<String>, host: impl Into<String>) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('DROP USER IF EXISTS ', QUOTE(?), '@', QUOTE(?))",
        user.into(),
        host.into()
    ))
}

pub(crate) fn grant_privilege(
    table: impl Into<String>,
    privilege: impl Into<String>,
    user: impl Into<String>,
    host: impl Into<String>,
) -> PreparedQuery {
    PreparedQuery(query!("SET @STATEMENT = CONCAT('GRANT ', ?, ' ON TABLE `', ?, '`.* TO ', QUOTE(?), '@', QUOTE(?))", privilege.into(), table.into(), user.into(), host.into()))
}

#[allow(unused)]
pub(crate) fn revoke_all_privileges(
    table: impl Into<String>,
    user: impl Into<String>,
    host: impl Into<String>,
) -> PreparedQuery {
    PreparedQuery(query!("SET @STATEMENT = CONCAT('REVOKE ALL PRIVILEGES ON `', ?, '`.* FROM ', QUOTE(?), '@', QUOTE(?))", table.into(), user.into(), host.into()))
}

#[allow(unused)]
pub(crate) fn grant_proxy(
    user: impl Into<String>,
    host: impl Into<String>,
    proxy_user: impl Into<String>,
    proxy_host: impl Into<String>,
) -> PreparedQuery {
    PreparedQuery(query!("SET @STATEMENT = CONCAT('GRANT PROXY ON ', QUOTE(?), '@', QUOTE(?), ' TO ', QUOTE(?), '@', QUOTE(?))", user.into(), host.into(), proxy_user.into(), proxy_host.into()))
}

#[allow(unused)]
pub(crate) fn create_database(name: impl Into<String>) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('CREATE DATABASE ', ?)",
        name.into()
    ))
}

#[allow(unused)]
pub(crate) fn drop_database(name: impl Into<String>) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('DROP DATABASE ', ?)",
        name.into()
    ))
}

#[allow(unused)]
pub(crate) fn flush_privileges() -> PreparedQuery {
    PreparedQuery(query!("SET @STATEMENT = 'FLUSH PRIVILEGES'"))
}

#[cfg(test)]
mod tests {
    use crate::sql::{DATABASE_MIGRATOR, MIGRATOR};

    #[test]
    fn check_migrations_found() {
        assert!(MIGRATOR.iter().len() > 0);
        assert!(DATABASE_MIGRATOR.iter().len() > 0);
    }
}
