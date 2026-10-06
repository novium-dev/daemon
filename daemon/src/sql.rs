// SPDX-License-Identifier: AGPL-3.0-only
#[cfg(feature = "databases")]
mod mariadb;

#[cfg(feature = "databases")]
pub use mariadb::*;
use sqlx::migrate::Migrator;
use sqlx::{MySql, Pool};

static MIGRATOR: Migrator = sqlx::migrate!();

pub async fn init(pool: &Pool<MySql>) -> Result<(), sqlx::Error> {
    MIGRATOR.run(pool).await?;

    Ok(())
}
