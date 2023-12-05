use std::sync::{Arc, OnceLock};

use anyhow::Result;
use once_cell::sync::Lazy;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};

use crate::config;

pub mod table;

static POSTGRES: Lazy<Arc<OnceLock<PostgresSQL>>> = Lazy::new(|| Arc::new(OnceLock::new()));

pub struct PostgresSQL {
    pub pool: PgPool,
}

pub(super) trait CopyIn: Send {
    fn to_csv(&self) -> String;
}

pub(super) async fn copy_in_raw(copy_in_query: &str, items: &[impl CopyIn + Send]) -> Result<u64> {
    let data: String = items.iter().map(CopyIn::to_csv).collect();
    let data_as_bytes = data.as_bytes();
    let mut conn = get_connection().acquire().await?;
    let mut writer = conn.copy_in_raw(copy_in_query).await?;

    writer.send(data_as_bytes).await?;

    Ok(writer.finish().await?)
}

impl PostgresSQL {
    pub fn new() -> PostgresSQL {
        let database_url = format!(
            "postgres://{}:{}@{}:{}/{}?application_name=stock_crawler_rust",
            config::SETTINGS.postgresql.user,
            config::SETTINGS.postgresql.password,
            config::SETTINGS.postgresql.host,
            config::SETTINGS.postgresql.port,
            config::SETTINGS.postgresql.db
        );
        let db = PgPoolOptions::new()
            .max_lifetime(None)
            .max_connections(1024)
            .connect_lazy(&database_url)
            .unwrap_or_else(|_| panic!("wrong database URL {}", database_url));

        Self { pool: db }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn tx(&self) -> Result<Transaction<'_, Postgres>> {
        Ok(self.pool().begin().await?)
    }
}

impl Default for PostgresSQL {
    fn default() -> Self {
        Self::new()
    }
}

fn get_postgresql() -> &'static PostgresSQL {
    POSTGRES.get_or_init(PostgresSQL::new)
}

pub fn get_connection() -> &'static PgPool {
    get_postgresql().pool()
}

pub async fn get_tx() -> Result<Transaction<'static, Postgres>> {
    get_postgresql().tx().await
}
