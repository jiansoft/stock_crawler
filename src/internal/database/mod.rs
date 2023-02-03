pub mod model;

use crate::config;
use once_cell::sync::Lazy;
use sqlx::{postgres::PgPoolOptions, PgPool};

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

/*pub async fn fetch_index() {
    let stmt = r#"
select category, "date", trading_volume, "transaction", trade_value, change, index,create_time, update_time
from index
order by "date" desc
limit 30 OFFSET 0;
        "#;
    let mut stream = sqlx::query_as::<_, model::index::Entity>(&stmt).fetch(&DB.db);

    let mut indices : HashMap<String, model::index::Entity> = HashMap::new();

    while let Some(row_result) = stream.next().await {
        if let Ok(row) = row_result {
            logging::info_file_async(format!("row.date {:?} row.index {:?}", row.date, row.index));
            indices.insert(row.date.to_string(),row);
        };
    }

    logging::info_file_async(format!("indices.len {:?}", indices.len()));
}

#[cfg(test)]
mod tests {

    use super::*;

    #[tokio::test]
    async fn test_fetch_index() {
        dotenv::dotenv().ok();
        fetch_index().await;
    }
}
*/
