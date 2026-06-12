use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Datelike, Local, TimeDelta};
use rust_decimal::Decimal;
use sqlx::{Row, postgres::PgQueryResult, postgres::PgRow};

use crate::{
    core::declare::Industry,
    core::logging,
    core::util::{self, map::Keyable},
    infra::database,
};

pub(crate) mod extension;
pub mod stock_exchange_market;
pub(crate) mod stock_index;
pub mod stock_ownership_details;
pub(crate) mod stock_word;

#[derive(sqlx::Type, sqlx::FromRow, Debug, Clone)]
/// 原表名 stocks
pub struct StockDbRow {
    /// 股票代號。
    pub stock_symbol: String,
    /// 股票名稱。
    pub name: String,
    /// 是否下市或停止交易。
    pub suspend_listing: bool,
    /// 每股淨值。
    pub net_asset_value_per_share: Decimal,
    /// 權值占比。
    pub weight: Decimal,
    /// 股東權益報酬率（ROE）。
    pub return_on_equity: Decimal,
    /// 建立時間。
    pub create_time: DateTime<Local>,
    /// 交易所的市場編號參考 StockExchangeMarket
    pub stock_exchange_market_id: i32,
    /// 股票的產業分類編號 stock_industry
    pub stock_industry_id: i32,
    /// 已發行股數
    pub issued_share: i64,
    /// 全體外資及陸資持有股數
    pub qfii_shares_held: i64,
    /// 全體外資及陸資持股比率
    pub qfii_share_holding_percentage: Decimal,
}

impl StockDbRow {
    /// 建立 `StockDbRow` 預設值。
    pub fn new() -> Self {
        StockDbRow {
            stock_symbol: "".to_string(),
            name: "".to_string(),
            suspend_listing: false,
            net_asset_value_per_share: Default::default(),
            weight: Default::default(),
            return_on_equity: Default::default(),
            create_time: Local::now(),
            stock_exchange_market_id: 0,
            stock_industry_id: 0,
            issued_share: 0,
            qfii_shares_held: 0,
            qfii_share_holding_percentage: Default::default(),
        }
    }

    /// 更新個股最新一季、近四季的EPS、ROE
    pub async fn update_eps_and_roe() -> Result<PgQueryResult> {
        let sql = r#"
WITH fs_data AS (
    SELECT
        row_number() OVER (
            PARTITION BY security_code
            ORDER BY year DESC, quarter DESC
        ) AS row_number,
        serial
    FROM
        financial_statement
    WHERE
        year IN ($1, $2)
        AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
),
relevant_fs_rows AS (
    SELECT
        fs_data.row_number,
        fs.security_code,
        fs.earnings_per_share,
        fs.net_asset_value_per_share,
        fs.return_on_equity
    FROM
        financial_statement fs
    JOIN
        fs_data ON fs_data.serial = fs.serial
    ORDER BY year DESC, quarter DESC
),
aggregated_eps AS (
    SELECT
        security_code,
        SUM(earnings_per_share) AS last_four_eps
    FROM
        relevant_fs_rows
    WHERE
        row_number <= 4
    GROUP BY
        security_code
)
UPDATE
    stocks
SET
    last_four_eps = agg.last_four_eps,
    last_one_eps = current_row.earnings_per_share,
    net_asset_value_per_share = current_row.net_asset_value_per_share,
    return_on_equity = current_row.return_on_equity
FROM
    relevant_fs_rows AS current_row
JOIN
    aggregated_eps AS agg ON current_row.security_code = agg.security_code
WHERE
    current_row.security_code = stocks.stock_symbol;
"#;
        let now = Local::now();
        let one_year_ago = now - TimeDelta::try_days(365).unwrap();

        sqlx::query(sql)
            .bind(now.year())
            .bind(one_year_ago.year())
            .execute(database::get_connection())
            .await
            .context("Failed to update_last_eps from database")
    }

    /// 取得所有股票
    pub async fn fetch() -> Result<Vec<StockDbRow>> {
        let sql = r#"
SELECT
    stock_symbol,
    "Name" AS name,
    "SuspendListing" AS suspend_listing,
    "CreateTime" AS create_time,
    net_asset_value_per_share,
    return_on_equity,
    weight,
    stock_exchange_market_id,
    stock_industry_id,
    issued_share,
    qfii_shares_held,
    qfii_share_holding_percentage
FROM
    stocks
ORDER BY
    stock_exchange_market_id,
    stock_industry_id;
"#;
        sqlx::query(sql)
            .try_map(|row: PgRow| {
                Ok(StockDbRow {
                    stock_symbol: row.try_get("stock_symbol")?,
                    net_asset_value_per_share: row.try_get("net_asset_value_per_share")?,
                    weight: row.try_get("weight")?,
                    name: row.try_get("name")?,
                    suspend_listing: row.try_get("suspend_listing")?,
                    create_time: row.try_get("create_time")?,
                    stock_exchange_market_id: row.try_get("stock_exchange_market_id")?,
                    stock_industry_id: row.try_get("stock_industry_id")?,
                    issued_share: row.try_get("issued_share")?,
                    qfii_shares_held: row.try_get("qfii_shares_held")?,
                    return_on_equity: row.try_get("return_on_equity")?,
                    qfii_share_holding_percentage: row.try_get("qfii_share_holding_percentage")?,
                })
            })
            .fetch_all(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to StockDbRow::fetch from database({}) because:{:?}",
                    database::redacted_postgresql_summary(),
                    why
                )
            })
    }

    /// 依現有 `stocks` 主檔重建搜尋索引。
    ///
    /// 用於直接執行 migration 匯入資料後，補建 `company_word` / `company_index`。
    pub async fn rebuild_search_indices() -> Result<()> {
        for stock in Self::fetch().await? {
            create_search_index(&stock.stock_symbol, &stock.name).await;
        }

        Ok(())
    }
}

/// <summary>
/// 為特定證券主檔建立或重新建立搜尋索引。
/// </summary>
pub async fn create_search_index(stock_symbol: &str, name: &str) {
    // 先刪除舊的數據
    if let Err(why) = stock_index::StockIndex::delete_by_stock_symbol(stock_symbol).await {
        logging::error_file_async(format!("{:#?}", why));
    }

    // 拆解股票名稱為單詞並加入股票代碼
    let mut words = util::text::split(name);
    words.push(stock_symbol.to_string());

    // 查詢已存在的單詞，轉成 hashmap 方便查詢
    let words_in_db = stock_word::StockWord::list_by_word(&words).await;
    let exist_words = match words_in_db {
        Ok(sw) => util::map::vec_to_hashmap(sw),
        Err(why) => {
            logging::error_file_async(format!("Failed to list_by_word because:{:#?}", why));
            return;
        }
    };

    for word in words {
        let mut stock_index_e = stock_index::StockIndex::new(stock_symbol.to_string());

        match exist_words.get(&word) {
            Some(w) => {
                //word 已存在資料庫了
                stock_index_e.word_id = w.word_id;
            }
            None => {
                let mut stock_word_e = stock_word::StockWord::new(word);
                match stock_word_e.upsert().await {
                    Ok(word_id) => {
                        stock_index_e.word_id = word_id;
                    }
                    Err(why) => {
                        logging::error_file_async(format!(
                            "Failed to insert stock word because:{:#?}",
                            why
                        ));
                        continue;
                    }
                }
            }
        }

        if let Err(why) = stock_index_e.insert().await {
            logging::error_file_async(format!("Failed to insert stock index because:{:#?}", why));
        }
    }
}

impl Keyable for StockDbRow {
    fn key(&self) -> String {
        self.stock_symbol.clone()
    }

    fn key_with_prefix(&self) -> String {
        format!("Stock:{}", self.key())
    }
}

impl Default for StockDbRow {
    fn default() -> Self {
        StockDbRow::new()
    }
}

impl From<StockDbRow> for crate::domain::registry::entity::Stock {
    fn from(row: StockDbRow) -> Self {
        crate::domain::registry::entity::Stock::reconstitute(
            row.stock_symbol,
            row.name,
            row.suspend_listing,
            row.net_asset_value_per_share,
            row.weight,
            row.return_on_equity,
            row.create_time,
            row.stock_exchange_market_id,
            row.stock_industry_id,
            row.issued_share,
            row.qfii_shares_held,
            row.qfii_share_holding_percentage,
        )
    }
}

impl From<&crate::domain::registry::entity::Stock> for StockDbRow {
    fn from(stock: &crate::domain::registry::entity::Stock) -> Self {
        StockDbRow {
            stock_symbol: stock.symbol().0.clone(),
            name: stock.name().to_string(),
            suspend_listing: stock.suspend_listing(),
            net_asset_value_per_share: stock.net_asset_value_per_share(),
            weight: stock.weight(),
            return_on_equity: stock.return_on_equity(),
            create_time: stock.created_time(),
            stock_exchange_market_id: stock.market_id(),
            stock_industry_id: stock.industry_id(),
            issued_share: stock.issued_share(),
            qfii_shares_held: stock.qfii_shares_held(),
            qfii_share_holding_percentage: stock.qfii_share_holding_percentage(),
        }
    }
}

impl From<crate::domain::registry::entity::Stock> for StockDbRow {
    fn from(stock: crate::domain::registry::entity::Stock) -> Self {
        StockDbRow::from(&stock)
    }
}



//let entity: Entity = fs.into(); // 或者 let entity = Entity::from(fs);

/// 取得未下市上市櫃每股淨值為零的股票
pub async fn fetch_net_asset_value_per_share_is_zero() -> Result<Vec<StockDbRow>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    s.return_on_equity,
    s.stock_exchange_market_id,
    s.stock_industry_id,
    s.weight,
    s.issued_share,
    s.qfii_shares_held,
    s.qfii_share_holding_percentage
FROM stocks AS s
WHERE s.stock_exchange_market_id in (2, 4)
    AND s."SuspendListing" = false
    AND s.stock_industry_id != $1
    AND s.net_asset_value_per_share = 0
"#;

    sqlx::query_as::<_, StockDbRow>(sql)
        .bind(Industry::ExchangeTradedFund.serial())
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_net_asset_value_per_share_is_zero from database")
}

/// 取得尚未有指定年度的季報的股票或者財報的每股淨值為零的股票
pub async fn fetch_stocks_without_financial_statement(
    year: i32,
    quarter: &str,
) -> Result<Vec<StockDbRow>> {
    let sql = r#"
SELECT
    s.stock_symbol,
    s."Name" AS name,
    s."SuspendListing" AS suspend_listing,
    s."CreateTime" AS create_time,
    s.net_asset_value_per_share,
    s.return_on_equity,
    s.stock_exchange_market_id,
    s.stock_industry_id,
    s.weight,
    s.issued_share,
    s.qfii_shares_held,
    s.qfii_share_holding_percentage
FROM stocks AS s
WHERE s.stock_exchange_market_id in(2, 4)
    AND s."SuspendListing" = false
    AND s.stock_industry_id != $3
    AND NOT EXISTS (
        SELECT 1
        FROM financial_statement f
        WHERE f.security_code = s.stock_symbol
        AND f.year = $1
        AND f.quarter = $2
        AND earnings_per_share > 0
    )
"#;

    sqlx::query_as::<_, StockDbRow>(sql)
        .bind(year)
        .bind(quarter)
        .bind(Industry::ExchangeTradedFund.serial())
        .fetch_all(database::get_connection())
        .await
        .context("Failed to fetch_stocks_without_financial_statement from database")
}

/// 是否為特別股
pub fn is_preference_shares(stock_symbol: &str) -> bool {
    stock_symbol
        .chars()
        .any(|c| c.is_ascii_uppercase() || c.is_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;

    use crate::core::logging;
    use crate::domain::registry::repository::StockRepository;

    use super::*;

    #[test]
    fn is_preference_shares_detects_letters_only() {
        assert!(!is_preference_shares("2330"));
        assert!(!is_preference_shares("0050"));
        assert!(is_preference_shares("2881A"));
        assert!(is_preference_shares("abc"));
        assert!(is_preference_shares("123b"));
    }

    #[test]
    fn stock_methods_key_tdr_and_stock_info_request() {
        let mut stock = StockDbRow::new();
        stock.stock_symbol = "9105".to_string();
        stock.name = "泰金寶-DR".to_string();
        stock.stock_exchange_market_id = 2;
        stock.stock_industry_id = 20;
        stock.net_asset_value_per_share = dec!(12.34);
        stock.suspend_listing = true;

        assert_eq!(stock.key(), "9105");
        assert_eq!(stock.key_with_prefix(), "Stock:9105");
    }

    #[test]
    fn stock_clone_preserves_all_fields() {
        let mut stock = StockDbRow::new();
        stock.stock_symbol = "2330".to_string();
        stock.name = "台積電".to_string();
        stock.suspend_listing = true;
        stock.net_asset_value_per_share = dec!(95.12);
        stock.weight = dec!(31.5);
        stock.return_on_equity = dec!(25.7);
        stock.stock_exchange_market_id = 2;
        stock.stock_industry_id = 24;
        stock.issued_share = 25_000;
        stock.qfii_shares_held = 12_000;
        stock.qfii_share_holding_percentage = dec!(48.5);

        let cloned = stock.clone();

        assert_eq!(cloned.stock_symbol, stock.stock_symbol);
        assert_eq!(cloned.name, stock.name);
        assert_eq!(cloned.suspend_listing, stock.suspend_listing);
        assert_eq!(
            cloned.net_asset_value_per_share,
            stock.net_asset_value_per_share
        );
        assert_eq!(cloned.weight, stock.weight);
        assert_eq!(cloned.return_on_equity, stock.return_on_equity);
        assert_eq!(cloned.create_time, stock.create_time);
        assert_eq!(
            cloned.stock_exchange_market_id,
            stock.stock_exchange_market_id
        );
        assert_eq!(cloned.stock_industry_id, stock.stock_industry_id);
        assert_eq!(cloned.issued_share, stock.issued_share);
        assert_eq!(cloned.qfii_shares_held, stock.qfii_shares_held);
        assert_eq!(
            cloned.qfii_share_holding_percentage,
            stock.qfii_share_holding_percentage
        );
    }



    // 此測試驗證防禦性 upsert 邏輯（當新傳入的市場或產業編號為 0 時，保留資料庫中原先正確的非零值）。
    #[tokio::test]
    async fn test_upsert_industry() {
        dotenv::dotenv().ok();

        // 若目前測試環境無法連接資料庫，則自動跳過，避免在無資料庫之開發環境下執行單元測試失敗
        if crate::infra::database::ping().await.is_err() {
            println!("跳過 test_upsert_industry：資料庫未啟動或無法連線");
            return;
        }

        let test_symbol = "__TEST_UPSERT_IND_9999__";
        let cleanup_sql = "DELETE FROM stocks WHERE stock_symbol = $1;";

        // 1. 確保乾淨的測試起點，先清除可能存在的舊測試資料
        sqlx::query(cleanup_sql)
            .bind(test_symbol)
            .execute(crate::infra::database::get_connection())
            .await
            .ok();

        let repo = crate::infra::database::repository::stock::PgStockRepository::new();

        // 2. 寫入初始的測試股票資料，並給予非零之市場與產業編號
        let seed = crate::domain::registry::entity::Stock::reconstitute(
            test_symbol.to_string(),
            "測試防禦更新股".to_string(),
            false,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Local::now(),
            4,
            15,
            0,
            0,
            Decimal::ZERO,
        );
        repo.save(&seed).await.expect("Failed to insert seed stock");

        // 3. 測試防禦邏輯：呼叫 upsert 時傳入 0 的市場與產業 ID
        let update_zero = crate::domain::registry::entity::Stock::reconstitute(
            test_symbol.to_string(),
            "測試防禦更新股_已更新".to_string(),
            false,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Local::now(),
            0,
            0,
            0,
            0,
            Decimal::ZERO,
        );
        repo.save(&update_zero)
            .await
            .expect("Failed to upsert zero fields");

        // 4. 驗證防禦更新：確認原先的非零值 (4 與 15) 被妥善保留，並未被 0 覆蓋
        let row1 = sqlx::query("SELECT \"Name\", stock_exchange_market_id, stock_industry_id FROM stocks WHERE stock_symbol = $1")
            .bind(test_symbol)
            .fetch_one(crate::infra::database::get_connection())
            .await
            .expect("Failed to fetch updated stock");
        assert_eq!(row1.get::<String, _>("Name"), "測試防禦更新股_已更新");
        assert_eq!(row1.get::<i32, _>("stock_exchange_market_id"), 4);
        assert_eq!(row1.get::<i32, _>("stock_industry_id"), 15);

        // 5. 測試正常覆寫：呼叫 upsert 時傳入新的非零之市場與產業 ID
        let update_new = crate::domain::registry::entity::Stock::reconstitute(
            test_symbol.to_string(),
            "測試防禦更新股_二次更新".to_string(),
            false,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            Local::now(),
            2,
            8,
            0,
            0,
            Decimal::ZERO,
        );
        repo.save(&update_new)
            .await
            .expect("Failed to upsert new fields");

        // 6. 驗證覆寫更新：確認市場與產業 ID 已經被正確更新為新的值 (2 與 8)
        let row2 = sqlx::query("SELECT \"Name\", stock_exchange_market_id, stock_industry_id FROM stocks WHERE stock_symbol = $1")
            .bind(test_symbol)
            .fetch_one(crate::infra::database::get_connection())
            .await
            .expect("Failed to fetch second updated stock");
        assert_eq!(row2.get::<String, _>("Name"), "測試防禦更新股_二次更新");
        assert_eq!(row2.get::<i32, _>("stock_exchange_market_id"), 2);
        assert_eq!(row2.get::<i32, _>("stock_industry_id"), 8);

        // 7. 清理測試資料，還原資料庫狀態
        sqlx::query(cleanup_sql)
            .bind(test_symbol)
            .execute(crate::infra::database::get_connection())
            .await
            .expect("Failed to cleanup test stock");
    }

    #[tokio::test]
    #[ignore]
    async fn test_update_last_eps() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 update_last_eps".to_string());
        match StockDbRow::update_eps_and_roe().await {
            Ok(_) => {}
            Err(why) => {
                logging::debug_file_async(format!("Failed to update_last_eps because: {:?}", why));
            }
        }

        logging::debug_file_async("結束 update_last_eps".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 StockDbRow::fetch".to_string());
        match StockDbRow::fetch().await {
            Ok(stocks) => {
                logging::debug_file_async(format!("stocks:{:#?}", stocks));
            }
            Err(why) => {
                logging::debug_file_async(format!("{:?}", why));
            }
        }
        logging::debug_file_async("結束 StockDbRow::fetch".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_net_asset_value_per_share_is_zero() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_net_asset_value_per_share_is_zero".to_string());
        match fetch_net_asset_value_per_share_is_zero().await {
            Ok(stocks) => {
                for e in stocks {
                    logging::debug_file_async(format!(
                        "{} {:?} ",
                        is_preference_shares(&e.stock_symbol),
                        e
                    ));
                }
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_net_asset_value_per_share_is_zero because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_net_asset_value_per_share_is_zero".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_fetch_stocks_without_financial_statement() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 fetch_stocks_without_financial_statement".to_string());
        match fetch_stocks_without_financial_statement(2022, "Q4").await {
            Ok(stocks) => {
                for e in stocks {
                    logging::debug_file_async(format!(
                        "{} {:?} ",
                        is_preference_shares(&e.stock_symbol),
                        e
                    ));
                }
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to fetch_stocks_without_financial_statement because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 fetch_stocks_without_financial_statement".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_create_index() {
        dotenv::dotenv().ok();
        create_search_index("2330", "台積電").await;
    }

    #[tokio::test]
    #[ignore]
    async fn test_rebuild_search_indices() {
        dotenv::dotenv().ok();
        logging::debug_file_async("開始 rebuild_search_indices".to_string());

        match StockDbRow::rebuild_search_indices().await {
            Ok(_) => {
                logging::debug_file_async("完成 rebuild_search_indices".to_string());
            }
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to rebuild_search_indices because: {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 rebuild_search_indices".to_string());
    }
}
