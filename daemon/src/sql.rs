// SPDX-License-Identifier: AGPL-3.0-only
use sqlx::migrate::Migrator;
use sqlx::mysql::MySqlArguments;
use sqlx::pool::PoolConnection;
use sqlx::query::Query;
use sqlx::{Connection as _, Executor, MySql, Pool, raw_sql};

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
pub struct MultiTransactionApply(Vec<(PreparedQuery, Vec<Query<'static, MySql, MySqlArguments>>)>);

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
