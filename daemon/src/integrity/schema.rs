// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use async_trait::async_trait;
use daemon_common::integrity::{
    IntegrityCategory, IntegrityFinding, IntegritySchema, IntegritySchemaTable,
};
use sqlx::{AssertSqlSafe, MySql, Pool, Row as _, query, query_scalar};

pub async fn dump(conn: &Pool<MySql>) -> Result<IntegritySchema, sqlx::Error> {
    let database: String = query_scalar("SELECT DATABASE()")
        .fetch_one(conn)
        .await?;

    let names: Vec<String> = query_scalar(
        "SELECT TABLE_NAME FROM information_schema.TABLES \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE' ORDER BY TABLE_NAME",
    )
    .fetch_all(conn)
    .await?;

    let mut tables = Vec::new();
    for name in names {
        if name.contains('`') {
            continue;
        }
        let row = query(AssertSqlSafe(format!("SHOW CREATE TABLE `{name}`")))
            .fetch_one(conn)
            .await?;
        let definition = row
            .try_get::<String, _>(1usize)
            .unwrap_or_else(|_err| String::new());
        tables.push(IntegritySchemaTable { name, definition });
    }

    Ok(IntegritySchema {
        engine: "mariadb".to_owned(),
        database,
        tables,
    })
}

pub struct SchemaDump;

#[async_trait]
impl Check for SchemaDump {
    fn id(&self) -> &'static str {
        "schema"
    }

    fn title(&self) -> &'static str {
        "Daemon database schema"
    }

    fn description(&self) -> &'static str {
        "Dumps the CREATE TABLE statement of every table in the daemon database so a developer \
         can compare a node against the migrations it should have run."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Schema
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let schema = dump(&ctx.conn).await?;

        Ok(schema
            .tables
            .into_iter()
            .map(|table| {
                IntegrityFinding::info(format!("Table {} of {}", table.name, schema.database))
                    .with_subject(table.name)
                    .with_detail("definition", table.definition)
            })
            .collect())
    }
}
