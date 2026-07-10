use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::Result;
use once_cell::sync::Lazy;
use sqlx::{PgPool, Postgres, Transaction, postgres::PgPoolOptions};

use crate::core::config;

/// 關聯式資料庫資料表與查詢邏輯。
pub mod table;

/// 領域倉儲實現。
pub mod repository;

/// 全程式共用的 PostgreSQL 連線池單例（singleton）。
///
/// - `Lazy`：第一次被使用時才建立，避免程式啟動就付出成本。
/// - `OnceLock`：保證多執行緒同時搶著初始化時，`PostgresSQL::new()` 也只會執行一次。
/// - 之後所有資料庫操作都透過 [`get_connection`] 取用同一個連線池，
///   而不是每次查詢都重新建立連線（建立連線非常昂貴）。
static POSTGRES: Lazy<Arc<OnceLock<PostgresSQL>>> = Lazy::new(|| Arc::new(OnceLock::new()));

/// PostgreSQL 連線池封裝。
///
/// 負責建立連線池並提供 transaction 入口，供 `database::table::*` 共享使用。
pub struct PostgresSQL {
    /// SQLx PostgreSQL 連線池實例。
    pub pool: PgPool,
}

/// 提供 `COPY ... FROM STDIN` 所需的 CSV 序列化能力。
///
/// 背景知識：PostgreSQL 的 `COPY` 是最快的批次匯入方式——資料以 CSV 文字流
/// 一次灌入，比逐筆 `INSERT` 快一個數量級以上。代價是它不支援
/// `ON CONFLICT`（upsert），遇到重複主鍵會直接整批失敗，
/// 因此使用前必須先確保目標範圍內沒有既有資料（例如同一 transaction 內先刪除）。
pub(super) trait CopyIn: Send {
    /// 將資料列轉成 PostgreSQL `COPY` 可接受的單行 CSV。
    fn to_csv(&self) -> String;
}

/// 以 PostgreSQL `COPY FROM STDIN` 批次寫入資料。
///
/// `items` 會先透過 [`CopyIn::to_csv`] 串接成一段 CSV，再一次送到資料庫。
/// 內部會自行從連線池取得一條「獨立連線」執行，也就是每次呼叫都是
/// 各自為政的自動提交（autocommit）操作，寫入一半失敗時無法回復先前的變更。
/// 若需要「刪除 + 寫入」這種必須同生共死的組合操作，
/// 請改用 [`copy_in_raw_on`] 搭配 transaction。
///
/// # Errors
/// 當取得連線、建立 copy writer、傳送資料或結束 copy 流程失敗時回傳錯誤。
pub(super) async fn copy_in_raw(copy_in_query: &str, items: &[impl CopyIn]) -> Result<u64> {
    // 從連線池借出一條連線；用完（離開此函式）會自動歸還給連線池。
    let mut conn = get_connection().acquire().await?;
    // 實際的 COPY 流程交給 copy_in_raw_on，讓「池連線」與「transaction 連線」共用同一份邏輯。
    copy_in_raw_on(&mut conn, copy_in_query, items).await
}

/// 在「指定連線」上以 PostgreSQL `COPY FROM STDIN` 批次寫入資料。
///
/// 與 [`copy_in_raw`] 的差別在於：這個版本不自己借連線，而是使用呼叫端傳入的
/// `conn`。當呼叫端把 transaction 的連線（`&mut *tx`）傳進來時，
/// COPY 寫入就會成為該 transaction 的一部分——commit 前外界看不到新資料，
/// 中途失敗則整批 rollback，不會留下「寫到一半」的狀態。
///
/// # Errors
/// 當建立 copy writer、傳送資料或結束 copy 流程失敗時回傳錯誤。
pub(super) async fn copy_in_raw_on(
    conn: &mut sqlx::PgConnection,
    copy_in_query: &str,
    items: &[impl CopyIn],
) -> Result<u64> {
    // 把每一筆資料轉成 CSV 字串後串接成一大段，一次送進 PostgreSQL 的 COPY 通道。
    let data: String = items.iter().map(CopyIn::to_csv).collect();
    let data_as_bytes = data.as_bytes();
    // 向資料庫宣告開始 COPY，取得可寫入的資料流（writer）。
    let mut writer = conn.copy_in_raw(copy_in_query).await?;

    writer.send(data_as_bytes).await?;

    // finish 會結束 COPY 並回傳實際寫入的資料列數。
    Ok(writer.finish().await?)
}

/// 回傳不含密碼的 PostgreSQL 連線摘要。
///
/// 錯誤訊息與測試輸出會使用這個摘要，避免把 `POSTGRESQL_PASSWORD`
/// 或設定檔中的密碼寫入 console/log。
pub(crate) fn redacted_postgresql_summary() -> String {
    format!(
        "PostgreSQL {{ host: {:?}, port: {}, user: {:?}, password: \"***\", db: {:?} }}",
        config::SETTINGS.postgresql.host,
        config::SETTINGS.postgresql.port,
        config::SETTINGS.postgresql.user,
        config::SETTINGS.postgresql.db
    )
}

impl PostgresSQL {
    /// 建立 PostgreSQL 連線池。
    ///
    /// 連線參數來自 `config::SETTINGS.postgresql`，並套用本專案的連線數與 timeout 設定。
    ///
    /// 注意：這裡使用 `connect_lazy`，表示此函式回傳時「尚未」真正連上資料庫，
    /// 第一次執行查詢時才會實際建立連線；因此設定錯誤（帳密、主機）
    /// 要到第一次查詢才會暴露，健康檢查請使用 [`ping`]。
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
            .unwrap_or_else(|_| {
                panic!(
                    "wrong database URL postgres://{}:***@{}:{}/{}?application_name=stock_crawler_rust",
                    config::SETTINGS.postgresql.user,
                    config::SETTINGS.postgresql.host,
                    config::SETTINGS.postgresql.port,
                    config::SETTINGS.postgresql.db
                )
            });

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

/// 檢查資料庫連線是否健康（Ping）。
///
/// 藉由對資料庫執行一個簡單的 `SELECT 1` 查詢來驗證連線是否正常。
///
/// # 錯誤
/// 若連線失敗、逾時或查詢執行失敗則回傳錯誤。
pub async fn ping() -> Result<()> {
    sqlx::query("SELECT 1").execute(get_connection()).await?;
    Ok(())
}
