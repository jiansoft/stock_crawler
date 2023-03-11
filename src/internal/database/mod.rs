pub mod model;

use crate::config;
use once_cell::sync::Lazy;
use sqlx::{postgres::PgPoolOptions, PgPool};

pub struct PostgreSQL {
    pub pool: PgPool,
}

impl PostgreSQL {
    pub fn new(database_url: &str) -> PostgreSQL {
        let db = PgPoolOptions::new()
            .max_connections(32)
            .connect_lazy(database_url)
            .unwrap_or_else(|_| panic!("wrong database URL {}", database_url));

        Self { pool: db }
    }
}

pub static DB: Lazy<PostgreSQL> = Lazy::new(|| {
    let db_url = format!(
        "postgres://{}:{}@{}:{}/{}",
        config::SETTINGS.postgresql.user,
        config::SETTINGS.postgresql.password,
        config::SETTINGS.postgresql.host,
        config::SETTINGS.postgresql.port,
        config::SETTINGS.postgresql.db
    );

    //let db_url = dotenv::var("DATABASE_URL").expect("DATABASE_URL must be set");
    PostgreSQL::new(&db_url)
});
