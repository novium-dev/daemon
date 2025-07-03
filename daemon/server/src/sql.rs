use sqlx::migrate::Migrator;
use sqlx::mysql::MySqlArguments;
use sqlx::pool::PoolConnection;
use sqlx::query::Query;
use sqlx::{Connection as _, Executor, MySql, Pool, query, raw_sql};

static MIGRATOR: Migrator = sqlx::migrate!();

pub struct PreparedQuery(Query<'static, MySql, MySqlArguments>);

impl PreparedQuery {
    pub async fn prepare<'a, E: Executor<'a, Database = MySql>>(
        self,
        executor: E,
    ) -> Result<ExecuteQuery, sqlx::Error> {
        self.0.execute(executor).await?;
        Ok(ExecuteQuery)
    }
}

pub struct ExecuteQuery;

impl ExecuteQuery {
    /// Definitely not cancel safe
    pub async fn execute<'a, E: Executor<'a, Database = MySql>>(
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
pub struct MultiTransactionApply(
    Vec<(PreparedQuery, Vec<Query<'static, MySql, MySqlArguments>>)>,
);

impl MultiTransactionApply {
    pub fn add(
        &mut self,
        main: PreparedQuery,
        transaction_query: Vec<Query<'static, MySql, MySqlArguments>>,
    ) {
        self.0.push((main, transaction_query));
    }

    pub async fn execute(
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

pub async fn init(pool: &Pool<MySql>) -> Result<(), sqlx::Error> {
    MIGRATOR.run(pool).await?;

    Ok(())
}

pub fn create_user(
    user: impl Into<String>,
    host: impl Into<String>,
    plugin: impl Into<String>,
    hash: impl Into<Option<String>>,
    lock: bool,
) -> PreparedQuery {
    let hash = hash.into();
    PreparedQuery(if let Some(hash) = hash {
        if lock {
            query!(
                "SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?), ' AS ', QUOTE(?), ' ACCOUNT LOCK')",
                user.into(),
                host.into(),
                plugin.into(),
                hash
            )
        } else {
            query!(
                "SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?), ' AS ', QUOTE(?))",
                user.into(),
                host.into(),
                plugin.into(),
                hash
            )
        }
    } else if lock {
        query!(
            "SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?), ' ACCOUNT LOCK')",
            user.into(),
            host.into(),
            plugin.into()
        )
    } else {
        query!(
            "SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ', QUOTE(?), '@', QUOTE(?), ' IDENTIFIED WITH ', QUOTE(?))",
            user.into(),
            host.into(),
            plugin.into()
        )
    })
}

pub fn proxy_user() -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('CREATE USER IF NOT EXISTS ''server''@''%'' IDENTIFIED WITH ', QUOTE(?))",
        crate::common::MARIADB_PLUGIN_NAME
    ))
}

pub fn drop_user(user: impl Into<String>, host: impl Into<String>) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('DROP USER IF EXISTS ', QUOTE(?), '@', QUOTE(?))",
        user.into(),
        host.into()
    ))
}

pub fn grant_privilege(
    table: impl Into<String>,
    privilege: impl Into<String>,
    user: impl Into<String>,
    host: impl Into<String>,
) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('GRANT ', ?, ' ON TABLE `', ?, '`.* TO ', QUOTE(?), '@', QUOTE(?))",
        privilege.into(),
        table.into(),
        user.into(),
        host.into()
    ))
}

pub fn revoke_all_privileges(
    table: impl Into<String>,
    user: impl Into<String>,
    host: impl Into<String>,
) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('REVOKE ALL PRIVILEGES ON `', ?, '`.* FROM ', QUOTE(?), '@', QUOTE(?))",
        table.into(),
        user.into(),
        host.into()
    ))
}

pub fn grant_proxy(
    user: impl Into<String>,
    host: impl Into<String>,
    proxy_user: impl Into<String>,
    proxy_host: impl Into<String>,
) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('GRANT PROXY ON ', QUOTE(?), '@', QUOTE(?), ' TO ', QUOTE(?), '@', QUOTE(?))",
        user.into(),
        host.into(),
        proxy_user.into(),
        proxy_host.into()
    ))
}

pub fn create_database(name: impl Into<String>) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('CREATE DATABASE ', ?)",
        name.into()
    ))
}

pub fn drop_database(name: impl Into<String>) -> PreparedQuery {
    PreparedQuery(query!(
        "SET @STATEMENT = CONCAT('DROP DATABASE ', ?)",
        name.into()
    ))
}

pub fn flush_privileges() -> PreparedQuery {
    PreparedQuery(query!("SET @STATEMENT = 'FLUSH PRIVILEGES'"))
}
