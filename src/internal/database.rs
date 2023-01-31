use crate::config;
use once_cell::sync::Lazy;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

pub struct PostgreSQL {
    pub db: PgPool,
}

impl PostgreSQL {
    pub fn new(database_url: String) -> PostgreSQL {
        let db = PgPoolOptions::new()
            .max_connections(32)
            .connect_lazy(&database_url)
            .expect(format!("wrong database URL {}", database_url).as_str());

        let db_context = Self { db };

        db_context
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
    PostgreSQL::new(db_url)
});
