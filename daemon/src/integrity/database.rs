// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use sqlx::{Row as _, query, query_scalar};
use std::collections::HashSet;

async fn schema_names(ctx: &IntegrityContext) -> Result<HashSet<String>, CheckError> {
    let rows = query("SELECT SCHEMA_NAME FROM information_schema.SCHEMATA")
        .fetch_all(&*ctx.conn)
        .await?;

    let mut names = HashSet::new();
    for row in rows {
        if let Ok(name) = row.try_get::<String, _>(0usize) {
            names.insert(name);
        }
    }
    Ok(names)
}

async fn account_names(
    ctx: &IntegrityContext,
) -> Result<Option<Vec<(String, String)>>, CheckError> {
    let Ok(rows) = query("SELECT User, Host, plugin FROM mysql.user")
        .fetch_all(&*ctx.conn)
        .await
    else {
        return Ok(None);
    };

    let mut accounts = Vec::new();
    for row in rows {
        let (Ok(user), Ok(plugin)) = (
            row.try_get::<String, _>(0usize),
            row.try_get::<String, _>(2usize),
        ) else {
            continue;
        };
        accounts.push((user, plugin));
    }
    Ok(Some(accounts))
}

pub struct MariadbDatabases;

#[async_trait]
impl Check for MariadbDatabases {
    fn id(&self) -> &'static str {
        "mariadb.databases"
    }

    fn title(&self) -> &'static str {
        "Server databases exist in MariaDB"
    }

    fn description(&self) -> &'static str {
        "Every row in the databases table must have a matching MariaDB schema, and every schema \
         carrying the daemon prefix must have a row. A schema without a row is invisible to the \
         panel but still consumes disk."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Database
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let databases = query!(r#"SELECT `id` AS `id: u32`, `name_suffix` FROM `databases`"#)
            .fetch_all(&*ctx.conn)
            .await?;

        let schemas = schema_names(ctx).await?;
        let prefix = crate::common::MARIADB_DATABASE_PREFIX;

        let mut expected = HashSet::new();
        let mut findings = Vec::new();

        for database in databases {
            let name = format!("{prefix}{}_{}", database.id, database.name_suffix);
            expected.insert(name.clone());
            if !schemas.contains(&name) {
                findings.push(
                    IntegrityFinding::error(format!(
                        "Database {} is registered, but the MariaDB schema does not exist",
                        database.id
                    ))
                    .with_subject(name)
                    .with_detail("database", database.id.to_string()),
                );
            }
        }

        for schema in &schemas {
            if schema.starts_with(prefix) && !expected.contains(schema) {
                findings.push(
                    IntegrityFinding::error(
                        "A MariaDB schema carries the daemon prefix but is not registered"
                            .to_owned(),
                    )
                    .with_subject(schema.clone()),
                );
            }
        }

        Ok(findings)
    }
}

pub struct MariadbUsers;

#[async_trait]
impl Check for MariadbUsers {
    fn id(&self) -> &'static str {
        "mariadb.users"
    }

    fn title(&self) -> &'static str {
        "Database users exist in MariaDB"
    }

    fn description(&self) -> &'static str {
        "Every database user and passwordless server grant must have a MariaDB account. The \
         shared proxy account and its auth plugin are only required once a server actually holds \
         a passwordless grant, so a node running without the plugin is left alone until then."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Database
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let users =
            query!(r#"SELECT `database` AS `database: u32`, `name_suffix` FROM `database_user`"#)
                .fetch_all(&*ctx.conn)
                .await?;

        let passwordless = query_scalar!(
            r#"SELECT DISTINCT `server` AS `server: u32` FROM `database_passwordless`"#
        )
        .fetch_all(&*ctx.conn)
        .await?;

        if users.is_empty() && passwordless.is_empty() {
            return Err(CheckError::Skipped(
                "No server database has been provisioned on this node, so nothing depends on the \
                 daemon's MariaDB accounts yet"
                    .to_owned(),
            ));
        }

        let Some(accounts) = account_names(ctx).await? else {
            return Err(CheckError::Skipped(
                "The daemon may not read mysql.user on this MariaDB instance".to_owned(),
            ));
        };

        let uses_passwordless = !passwordless.is_empty();

        let present: HashSet<&str> = accounts
            .iter()
            .map(|(user, _plugin)| user.as_str())
            .collect();
        let user_prefix = crate::common::MARIADB_USER_PREFIX;

        let mut expected = HashSet::new();
        let mut findings = Vec::new();

        for user in users {
            let name = format!("{user_prefix}{}_{}", user.database, user.name_suffix);
            expected.insert(name.clone());
            if !present.contains(name.as_str()) {
                findings.push(
                    IntegrityFinding::error(format!(
                        "User {} of database {} is registered, but the MariaDB account is missing",
                        user.name_suffix, user.database
                    ))
                    .with_subject(name)
                    .with_detail("database", user.database.to_string()),
                );
            }
        }

        for server in passwordless {
            let name = format!("server_{server}");
            expected.insert(name.clone());
            if !present.contains(name.as_str()) {
                findings.push(
                    IntegrityFinding::error(format!(
                        "Server {server} has passwordless database access, but its MariaDB account is missing"
                    ))
                    .with_subject(name)
                    .with_detail("server", server.to_string()),
                );
            }
        }

        for (user, plugin) in &accounts {
            if user == "server" {
                if plugin != crate::common::MARIADB_PLUGIN_NAME {
                    findings.push(
                        IntegrityFinding::error(format!(
                            "The shared proxy account authenticates with {plugin} instead of {}",
                            crate::common::MARIADB_PLUGIN_NAME
                        ))
                        .with_subject(user.clone()),
                    );
                }
                continue;
            }
            let daemon_owned = user.starts_with(user_prefix) || user.starts_with("server_");
            if daemon_owned && !expected.contains(user.as_str()) {
                findings.push(
                    IntegrityFinding::error(
                        "A MariaDB account carries a daemon prefix but is not registered"
                            .to_owned(),
                    )
                    .with_subject(user.clone()),
                );
            }
        }

        if !uses_passwordless {
            return Ok(findings);
        }

        if !present.contains("server") {
            findings.push(IntegrityFinding::error(
                "Servers on this node have passwordless database access, but the shared proxy \
                 account does not exist; none of them can connect"
                    .to_owned(),
            ));
        }

        let plugin_loaded =
            query("SELECT PLUGIN_STATUS FROM information_schema.PLUGINS WHERE PLUGIN_NAME = ?")
                .bind(crate::common::MARIADB_PLUGIN_NAME)
                .fetch_optional(&*ctx.conn)
                .await?;

        match plugin_loaded {
            None => findings.push(IntegrityFinding::error(format!(
                "Servers on this node have passwordless database access, but the authentication \
                 plugin {} is not installed in MariaDB",
                crate::common::MARIADB_PLUGIN_NAME
            ))),
            Some(row) => {
                let status = row
                    .try_get::<String, _>(0usize)
                    .unwrap_or_else(|_err| "UNKNOWN".to_owned());
                if !status.eq_ignore_ascii_case("ACTIVE") {
                    findings.push(
                        IntegrityFinding::error(format!(
                            "The authentication plugin {} is installed but not active",
                            crate::common::MARIADB_PLUGIN_NAME
                        ))
                        .with_detail("status", status),
                    );
                }
            }
        }

        Ok(findings)
    }
}
