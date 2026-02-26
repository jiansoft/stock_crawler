use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use once_cell::sync::Lazy;
use sqlx::{postgres::PgPoolOptions, PgPool, Postgres, Transaction};

use crate::config;

pub mod table;

static POSTGRES: Lazy<Arc<OnceLock<PostgresSQL>>> = Lazy::new(|| Arc::new(OnceLock::new()));

/// PostgreSQL 連線池封裝。
///
/// 負責建立連線池並提供 transaction 入口，供 `database::table::*` 共享使用。
pub struct PostgresSQL {
    /// SQLx PostgreSQL 連線池實例。
    pub pool: PgPool,
}

/// 提供 `COPY ... FROM STDIN` 所需的 CSV 序列化能力。
pub(super) trait CopyIn: Send {
    /// 將資料列轉成 PostgreSQL `COPY` 可接受的單行 CSV。
    fn to_csv(&self) -> String;
}

/// 以 PostgreSQL `COPY FROM STDIN` 批次寫入資料。
///
/// `items` 會先透過 [`CopyIn::to_csv`] 串接成一段 CSV，再一次送到資料庫。
///
/// # Errors
/// 當取得連線、建立 copy writer、傳送資料或結束 copy 流程失敗時回傳錯誤。
pub(super) async fn copy_in_raw(copy_in_query: &str, items: &[impl CopyIn]) -> Result<u64> {
    let data: String = items.iter().map(CopyIn::to_csv).collect();
    let data_as_bytes = data.as_bytes();
    let mut conn = get_connection().acquire().await?;
    let mut writer = conn.copy_in_raw(copy_in_query).await?;

    writer.send(data_as_bytes).await?;

    Ok(writer.finish().await?)
}

impl PostgresSQL {
    /// 建立 PostgreSQL 連線池。
    ///
    /// 連線參數來自 `config::SETTINGS.postgresql`，並套用本專案的連線數與 timeout 設定。
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
            .max_lifetime(Some(Duration::from_secs(1800))) // 30 分鐘
            .max_connections(20) // 個人專案降低連接數
            .min_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .idle_timeout(Some(Duration::from_secs(600))) // 10 分鐘
            .connect_lazy(&database_url)
            .unwrap_or_else(|_| panic!("wrong database URL {}", database_url));

        Self { pool: db }
    }

    /// 取得連線池參考。
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// 從目前連線池建立一筆 transaction。
    ///
    /// # Errors
    /// 當 `BEGIN` 失敗時回傳錯誤。
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

/// 取得全域 PostgreSQL 連線池。
pub fn get_connection() -> &'static PgPool {
    get_postgresql().pool()
}

/// 從全域 PostgreSQL 連線池建立 transaction。
///
/// # Errors
/// 當無法成功建立 transaction 時回傳錯誤。
pub async fn get_tx() -> Result<Transaction<'static, Postgres>> {
    get_postgresql().tx().await
}
