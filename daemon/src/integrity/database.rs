// SPDX-License-Identifier: AGPL-3.0-only
use super::{Check, CheckError, IntegrityContext};
use async_trait::async_trait;
use daemon_common::integrity::{IntegrityCategory, IntegrityFinding};
use daemon_common::service::database::DatabaseUserPermissions;
use sqlx::{query, query_scalar};
use std::collections::{HashMap, HashSet};

async fn schema_names(ctx: &IntegrityContext) -> Result<HashSet<String>, CheckError> {
    Ok(query_scalar!(
        r#"SELECT `SCHEMA_NAME` AS `name!: String` FROM `information_schema`.`SCHEMATA`"#
    )
    .fetch_all(&*ctx.conn)
    .await?
    .into_iter()
    .collect())
}

async fn account_names(ctx: &IntegrityContext) -> Option<Vec<(String, String)>> {
    let rows = query!(
        r#"SELECT `User` AS `user!: String`, `plugin` AS `plugin!: String` FROM `mysql`.`user`"#
    )
    .fetch_all(&*ctx.conn)
    .await
    .ok()?;

    Some(rows.into_iter().map(|row| (row.user, row.plugin)).collect())
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
                        "A MariaDB schema carries the daemon prefix but is not registered",
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

        let Some(accounts) = account_names(ctx).await else {
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
                        "A MariaDB account carries a daemon prefix but is not registered",
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
                 account does not exist; none of them can connect",
            ));
        }

        let plugin_status = query_scalar!(
            r#"SELECT `PLUGIN_STATUS` AS `status!: String` FROM `information_schema`.`PLUGINS`
               WHERE `PLUGIN_NAME` = ?"#,
            crate::common::MARIADB_PLUGIN_NAME
        )
        .fetch_optional(&*ctx.conn)
        .await?;

        match plugin_status {
            None => findings.push(IntegrityFinding::error(format!(
                "Servers on this node have passwordless database access, but the authentication \
                 plugin {} is not installed in MariaDB",
                crate::common::MARIADB_PLUGIN_NAME
            ))),
            Some(status) if !status.eq_ignore_ascii_case("ACTIVE") => findings.push(
                IntegrityFinding::error(format!(
                    "The authentication plugin {} is installed but not active",
                    crate::common::MARIADB_PLUGIN_NAME
                ))
                .with_detail("status", status),
            ),
            Some(_status) => {}
        }

        Ok(findings)
    }
}

fn grantee_user(grantee: &str) -> Option<&str> {
    grantee.split('\'').nth(1usize)
}

pub struct MariadbGrants;

#[async_trait]
impl Check for MariadbGrants {
    fn id(&self) -> &'static str {
        "mariadb.grants"
    }

    fn title(&self) -> &'static str {
        "Database accounts hold the privileges they were granted"
    }

    fn description(&self) -> &'static str {
        "Every database user and passwordless server grant records which privileges it should \
         have on its schema; MariaDB must agree. A passwordless server additionally needs the \
         shared proxy account to be allowed to impersonate it, or its connections are refused."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Database
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let users = query!(
            r#"SELECT `u`.`database` AS `database: u32`, `u`.`name_suffix`,
                      `u`.`permissions`, `d`.`name_suffix` AS `db_suffix`
               FROM `database_user` `u`
               JOIN `databases` `d` ON `d`.`id` = `u`.`database`"#
        )
        .fetch_all(&*ctx.conn)
        .await?;

        let passwordless = query!(
            r#"SELECT `p`.`server` AS `server: u32`, `p`.`permissions`,
                      `p`.`database` AS `database: u32`, `d`.`name_suffix` AS `db_suffix`
               FROM `database_passwordless` `p`
               JOIN `databases` `d` ON `d`.`id` = `p`.`database`"#
        )
        .fetch_all(&*ctx.conn)
        .await?;

        if users.is_empty() && passwordless.is_empty() {
            return Err(CheckError::Skipped(
                "No server database has been provisioned on this node".to_owned(),
            ));
        }

        let Ok(rows) = query!(
            r#"SELECT `GRANTEE` AS `grantee!: String`, `TABLE_SCHEMA` AS `schema!: String`,
                      `PRIVILEGE_TYPE` AS `privilege!: String`
               FROM `information_schema`.`SCHEMA_PRIVILEGES`"#
        )
        .fetch_all(&*ctx.conn)
        .await
        else {
            return Err(CheckError::Skipped(
                "The daemon may not read the schema privileges on this MariaDB instance".to_owned(),
            ));
        };

        let mut granted: HashMap<(String, String), HashSet<String>> = HashMap::new();
        for row in rows {
            let Some(user) = grantee_user(&row.grantee) else {
                continue;
            };
            granted
                .entry((user.to_owned(), row.schema))
                .or_default()
                .insert(row.privilege.to_uppercase());
        }

        let database_prefix = crate::common::MARIADB_DATABASE_PREFIX;
        let user_prefix = crate::common::MARIADB_USER_PREFIX;
        let mut findings = Vec::new();

        for user in users {
            let account = format!("{user_prefix}{}_{}", user.database, user.name_suffix);
            let schema = format!("{database_prefix}{}_{}", user.database, user.db_suffix);
            compare_privileges(&mut findings, &granted, &account, &schema, user.permissions);
        }

        for entry in passwordless {
            let account = format!("server_{}", entry.server);
            let schema = format!("{database_prefix}{}_{}", entry.database, entry.db_suffix);
            compare_privileges(
                &mut findings,
                &granted,
                &account,
                &schema,
                entry.permissions,
            );
        }

        Ok(findings)
    }
}

fn compare_privileges(
    findings: &mut Vec<IntegrityFinding>,
    granted: &HashMap<(String, String), HashSet<String>>,
    account: &str,
    schema: &str,
    permissions: i8,
) {
    let expected: HashSet<String> = DatabaseUserPermissions::from_bits_retain(permissions)
        .iter_names()
        .map(|(name, _flag)| name.to_uppercase())
        .collect();

    let actual = granted
        .get(&(account.to_owned(), schema.to_owned()))
        .cloned()
        .unwrap_or_default();

    let mut missing: Vec<&String> = expected.difference(&actual).collect();
    let mut extra: Vec<&String> = actual.difference(&expected).collect();
    missing.sort();
    extra.sort();

    if !missing.is_empty() {
        findings.push(
            IntegrityFinding::error(format!(
                "{account} is missing privileges MariaDB should have granted it on {schema}"
            ))
            .with_subject(account)
            .with_detail("schema", schema)
            .with_detail(
                "missing",
                missing
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        );
    }

    if !extra.is_empty() {
        findings.push(
            IntegrityFinding::warning(format!(
                "{account} holds privileges on {schema} that the daemon never granted it"
            ))
            .with_subject(account)
            .with_detail("schema", schema)
            .with_detail(
                "unexpected",
                extra
                    .iter()
                    .map(|name| name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
        );
    }
}

pub struct MariadbProxyGrants;

#[async_trait]
impl Check for MariadbProxyGrants {
    fn id(&self) -> &'static str {
        "mariadb.proxy_grants"
    }

    fn title(&self) -> &'static str {
        "The shared proxy account may impersonate every passwordless server"
    }

    fn description(&self) -> &'static str {
        "Passwordless database access works by letting the shared proxy account impersonate the \
         per server account. Without that PROXY grant the server authenticates but is refused."
    }

    fn category(&self) -> IntegrityCategory {
        IntegrityCategory::Database
    }

    async fn run(&self, ctx: &IntegrityContext) -> Result<Vec<IntegrityFinding>, CheckError> {
        let servers = query_scalar!(
            r#"SELECT DISTINCT `server` AS `server: u32` FROM `database_passwordless`"#
        )
        .fetch_all(&*ctx.conn)
        .await?;

        if servers.is_empty() {
            return Err(CheckError::Skipped(
                "No server on this node uses passwordless database access".to_owned(),
            ));
        }

        let Ok(rows) = query!(
            r#"SELECT `User` AS `user!: String`, `Proxied_user` AS `proxied!: String`
               FROM `mysql`.`proxies_priv`"#
        )
        .fetch_all(&*ctx.conn)
        .await
        else {
            return Err(CheckError::Skipped(
                "The daemon may not read mysql.proxies_priv on this MariaDB instance".to_owned(),
            ));
        };

        let proxies: HashSet<(String, String)> = rows
            .into_iter()
            .map(|row| (row.user, row.proxied))
            .collect();

        let mut findings = Vec::new();
        for server in servers {
            let account = format!("server_{server}");
            if !proxies.contains(&("server".to_owned(), account.clone())) {
                findings.push(
                    IntegrityFinding::error(format!(
                        "The shared proxy account may not impersonate {account}, so server \
                         {server} cannot use its passwordless database access"
                    ))
                    .with_subject(account)
                    .with_detail("server", server.to_string()),
                );
            }
        }

        Ok(findings)
    }
}
