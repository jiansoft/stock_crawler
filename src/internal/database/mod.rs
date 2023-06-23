pub mod table;

use crate::internal::config;
use anyhow::*;
use once_cell::sync::Lazy;
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::sync::{Arc, OnceLock};

static POSTGRES: Lazy<Arc<OnceLock<PostgresSQL>>> = Lazy::new(|| Arc::new(OnceLock::new()));

pub struct PostgresSQL {
    pub pool: PgPool,
}

impl PostgresSQL {
    pub fn new() -> PostgresSQL {
        let database_url = format!(
            "postgres://{}:{}@{}:{}/{}",
            config::SETTINGS.postgresql.user,
            config::SETTINGS.postgresql.password,
            config::SETTINGS.postgresql.host,
            config::SETTINGS.postgresql.port,
            config::SETTINGS.postgresql.db
        );
        let db = PgPoolOptions::new()
            .max_lifetime(None)
            .max_connections(32)
            .connect_lazy(&database_url)
            .unwrap_or_else(|_| panic!("wrong database URL {}", database_url));

        Self { pool: db }
    }
}

impl Default for PostgresSQL {
    fn default() -> Self {
        Self::new()
    }
}

pub fn get_pool() -> Result<&'static PgPool> {
    Ok(&POSTGRES.get_or_init(PostgresSQL::new).pool)
}
