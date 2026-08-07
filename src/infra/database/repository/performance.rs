use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::NaiveDate;
use sqlx::Row;

use crate::domain::performance::entity::{
    CagrCoverage, CagrMetric, CagrPeriod, StockCagr as DomainStockCagr,
};
use crate::domain::performance::query::{
    CagrRankingItem, CagrRankingPage, CagrRankingQuery, CagrSortKey,
};
use crate::domain::performance::repository::CagrRepository;
use crate::infra::database;
use crate::infra::database::table::performance::stock_cagr::StockCagr as TableStockCagr;

/// 單批寫入的最大列數。
///
/// PostgreSQL 單一敘述最多 65535 個參數，本表以 26 個陣列參數承載任意列數，
/// 參數個數不受列數影響；限制批量的目的是控制單次網路封包與伺服器端的
/// 記憶體用量（全市場 × 8 期間約 1.8 萬列）。
const SAVE_BATCH_SIZE: usize = 2_000;

/// 讀取用的欄位清單。
///
/// 與 [`TableStockCagr`] 的欄位順序一致；集中於此避免各查詢各寫一份而失去同步。
const SELECT_COLUMNS: &str = r#"
    date, stock_symbol, period, base_date, base_price, end_price, years,
    price_end_shares, price_end_value, price_return_pct, price_cagr_pct,
    total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct,
    reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct,
    first_quote_date, shortfall_days, data_complete, has_anomaly, dividend_events
"#;

/// 基於 PostgreSQL 的每日 CAGR 倉儲實現 (PgCagrRepository)。
///
/// 對應資料表 `public.stock_cagr`，主鍵為 `(date, stock_symbol, period)`。
pub struct PgCagrRepository;

impl PgCagrRepository {
    /// 建立新的 PgCagrRepository 實例。
    pub fn new() -> Self {
        PgCagrRepository
    }
}

impl Default for PgCagrRepository {
    fn default() -> Self {
        Self::new()
    }
}

/// 依報酬口徑取得對應的年化報酬率欄位名稱。
///
/// 回傳值是編譯期字面值，**不是**呼叫端傳入的字串——排序鍵必須以白名單
/// 映射，任何把外部輸入拼進 SQL 的寫法都是注入破口。
fn cagr_column(metric: CagrMetric) -> &'static str {
    match metric {
        CagrMetric::Price => "price_cagr_pct",
        CagrMetric::Total => "total_cagr_pct",
        CagrMetric::Reinvested => "reinv_cagr_pct",
    }
}

/// 依報酬口徑取得對應的區間總報酬率欄位名稱。
///
/// 同 [`cagr_column`]：回傳編譯期字面值，排序鍵一律走白名單映射。
fn return_column(metric: CagrMetric) -> &'static str {
    match metric {
        CagrMetric::Price => "price_return_pct",
        CagrMetric::Total => "total_return_pct",
        CagrMetric::Reinvested => "reinv_return_pct",
    }
}

/// 依（排序鍵, 口徑）取得排行榜排序所用的欄位名稱。
///
/// 兩個維度各自是封閉列舉，組合後仍只映射到六個編譯期字面值；
/// 呼叫端無從提供任何 SQL 片段。
fn sort_column(sort: CagrSortKey, metric: CagrMetric) -> &'static str {
    match sort {
        CagrSortKey::Cagr => cagr_column(metric),
        CagrSortKey::TotalReturn => return_column(metric),
    }
}

/// 排行榜查詢的共用 FROM／WHERE 片段（不含排序鍵相關條件）。
///
/// 綁定參數：`$1` 基準日、`$2` 期間代碼、`$3` 市場編號（NULL 不篩選）、
/// `$4` 產業編號（NULL 不篩選）、`$5` 關鍵字 ILIKE 樣式（NULL 不篩選）。
/// 排行榜與總筆數共用同一份條件，避免兩者因條件不同步而讓分頁錯亂。
const RANKING_FROM_WHERE: &str = r#"
FROM stock_cagr c
JOIN stocks s ON s.stock_symbol = c.stock_symbol
WHERE c.date = $1
  AND c.period = $2
  AND ($3::int IS NULL OR s.stock_exchange_market_id = $3)
  AND ($4::int IS NULL OR s.stock_industry_id = $4)
  AND ($5::text IS NULL OR c.stock_symbol ILIKE $5 OR s."Name" ILIKE $5)
"#;

/// 排行榜查詢的欄位清單（帶 `c.` 前綴）。
///
/// 與 [`SELECT_COLUMNS`] 內容相同，但因 JOIN `stocks` 後 `stock_symbol`
/// 會有歧義，必須逐欄限定來源資料表。
const RANKING_SELECT_COLUMNS: &str = r#"
    c.date, c.stock_symbol, c.period, c.base_date, c.base_price, c.end_price, c.years,
    c.price_end_shares, c.price_end_value, c.price_return_pct, c.price_cagr_pct,
    c.total_shares, c.total_cash, c.total_end_value, c.total_return_pct, c.total_cagr_pct,
    c.reinv_shares, c.reinv_cash, c.reinv_end_value, c.reinv_return_pct, c.reinv_cagr_pct,
    c.first_quote_date, c.shortfall_days, c.data_complete, c.has_anomaly, c.dividend_events
"#;

/// 排行榜查詢的資料列：CAGR 主體加上股票名稱、產業分類與名次。
#[derive(sqlx::FromRow)]
struct RankingRow {
    /// CAGR 計算結果本體。
    #[sqlx(flatten)]
    cagr: TableStockCagr,
    /// 股票名稱。
    name: String,
    /// 產業分類編號。
    industry_id: i32,
    /// 名次；資料不足（排序鍵為 NULL）者為 `None`。
    rank: Option<i64>,
}

/// 將關鍵字轉成 ILIKE 樣式，並跳脫萬用字元。
///
/// 使用者輸入的 `%` 與 `_` 應該當成字面字元比對；不跳脫的話輸入一個 `%`
/// 就會匹配全市場，看起來像是篩選失效。
fn keyword_pattern(keyword: &str) -> String {
    let escaped = keyword
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

/// 將資料列批次還原為領域實體，並依期間長度由短至長排序。
fn rows_to_domain(rows: Vec<TableStockCagr>) -> Result<Vec<DomainStockCagr>> {
    rows.into_iter().map(|row| row.to_domain()).collect()
}

#[async_trait]
impl CagrRepository for PgCagrRepository {
    /// 批次 upsert 指定基準日的 CAGR 計算結果。
    ///
    /// 刻意不使用 `COPY`：本專案的 [`CopyIn`](crate::infra::database) 無法處理
    /// `ON CONFLICT`，同日重跑會整批因主鍵重複而失敗。改以 `UNNEST` 展開
    /// 26 個陣列參數做多列插入，兼顧「一次往返寫入數千列」與 upsert 語意。
    async fn save_batch(&self, records: &[DomainStockCagr]) -> Result<u64> {
        if records.is_empty() {
            return Ok(0);
        }

        let sql = r#"
INSERT INTO stock_cagr (
    date, stock_symbol, period, base_date, base_price, end_price, years,
    price_end_shares, price_end_value, price_return_pct, price_cagr_pct,
    total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct,
    reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct,
    first_quote_date, shortfall_days, data_complete, has_anomaly, dividend_events
)
SELECT * FROM UNNEST(
    $1::date[], $2::varchar[], $3::varchar[], $4::date[], $5::numeric[], $6::numeric[], $7::numeric[],
    $8::numeric[], $9::numeric[], $10::numeric[], $11::numeric[],
    $12::numeric[], $13::numeric[], $14::numeric[], $15::numeric[], $16::numeric[],
    $17::numeric[], $18::numeric[], $19::numeric[], $20::numeric[], $21::numeric[],
    $22::date[], $23::int4[], $24::bool[], $25::bool[], $26::int4[]
) AS t(
    date, stock_symbol, period, base_date, base_price, end_price, years,
    price_end_shares, price_end_value, price_return_pct, price_cagr_pct,
    total_shares, total_cash, total_end_value, total_return_pct, total_cagr_pct,
    reinv_shares, reinv_cash, reinv_end_value, reinv_return_pct, reinv_cagr_pct,
    first_quote_date, shortfall_days, data_complete, has_anomaly, dividend_events
)
ON CONFLICT (date, stock_symbol, period) DO UPDATE SET
    base_date = EXCLUDED.base_date,
    base_price = EXCLUDED.base_price,
    end_price = EXCLUDED.end_price,
    years = EXCLUDED.years,
    price_end_shares = EXCLUDED.price_end_shares,
    price_end_value = EXCLUDED.price_end_value,
    price_return_pct = EXCLUDED.price_return_pct,
    price_cagr_pct = EXCLUDED.price_cagr_pct,
    total_shares = EXCLUDED.total_shares,
    total_cash = EXCLUDED.total_cash,
    total_end_value = EXCLUDED.total_end_value,
    total_return_pct = EXCLUDED.total_return_pct,
    total_cagr_pct = EXCLUDED.total_cagr_pct,
    reinv_shares = EXCLUDED.reinv_shares,
    reinv_cash = EXCLUDED.reinv_cash,
    reinv_end_value = EXCLUDED.reinv_end_value,
    reinv_return_pct = EXCLUDED.reinv_return_pct,
    reinv_cagr_pct = EXCLUDED.reinv_cagr_pct,
    first_quote_date = EXCLUDED.first_quote_date,
    shortfall_days = EXCLUDED.shortfall_days,
    data_complete = EXCLUDED.data_complete,
    has_anomaly = EXCLUDED.has_anomaly,
    dividend_events = EXCLUDED.dividend_events,
    updated_time = now();
"#;

        let mut affected: u64 = 0;

        for chunk in records.chunks(SAVE_BATCH_SIZE) {
            let len = chunk.len();
            let mut dates = Vec::with_capacity(len);
            let mut symbols = Vec::with_capacity(len);
            let mut periods = Vec::with_capacity(len);
            let mut base_dates = Vec::with_capacity(len);
            let mut base_prices = Vec::with_capacity(len);
            let mut end_prices = Vec::with_capacity(len);
            let mut years = Vec::with_capacity(len);
            let mut price_end_shares = Vec::with_capacity(len);
            let mut price_end_values = Vec::with_capacity(len);
            let mut price_return_pcts = Vec::with_capacity(len);
            let mut price_cagr_pcts = Vec::with_capacity(len);
            let mut total_shares = Vec::with_capacity(len);
            let mut total_cashes = Vec::with_capacity(len);
            let mut total_end_values = Vec::with_capacity(len);
            let mut total_return_pcts = Vec::with_capacity(len);
            let mut total_cagr_pcts = Vec::with_capacity(len);
            let mut reinv_shares = Vec::with_capacity(len);
            let mut reinv_cashes = Vec::with_capacity(len);
            let mut reinv_end_values = Vec::with_capacity(len);
            let mut reinv_return_pcts = Vec::with_capacity(len);
            let mut reinv_cagr_pcts = Vec::with_capacity(len);
            let mut first_quote_dates = Vec::with_capacity(len);
            let mut shortfall_days = Vec::with_capacity(len);
            let mut data_completes = Vec::with_capacity(len);
            let mut has_anomalies = Vec::with_capacity(len);
            let mut dividend_events = Vec::with_capacity(len);

            for record in chunk {
                let row = TableStockCagr::from(record);
                dates.push(row.date);
                symbols.push(row.stock_symbol);
                periods.push(row.period);
                base_dates.push(row.base_date);
                base_prices.push(row.base_price);
                end_prices.push(row.end_price);
                years.push(row.years);
                price_end_shares.push(row.price_end_shares);
                price_end_values.push(row.price_end_value);
                price_return_pcts.push(row.price_return_pct);
                price_cagr_pcts.push(row.price_cagr_pct);
                total_shares.push(row.total_shares);
                total_cashes.push(row.total_cash);
                total_end_values.push(row.total_end_value);
                total_return_pcts.push(row.total_return_pct);
                total_cagr_pcts.push(row.total_cagr_pct);
                reinv_shares.push(row.reinv_shares);
                reinv_cashes.push(row.reinv_cash);
                reinv_end_values.push(row.reinv_end_value);
                reinv_return_pcts.push(row.reinv_return_pct);
                reinv_cagr_pcts.push(row.reinv_cagr_pct);
                first_quote_dates.push(row.first_quote_date);
                shortfall_days.push(row.shortfall_days);
                data_completes.push(row.data_complete);
                has_anomalies.push(row.has_anomaly);
                dividend_events.push(row.dividend_events);
            }

            let result = sqlx::query(sql)
                .bind(&dates)
                .bind(&symbols)
                .bind(&periods)
                .bind(&base_dates)
                .bind(&base_prices)
                .bind(&end_prices)
                .bind(&years)
                .bind(&price_end_shares)
                .bind(&price_end_values)
                .bind(&price_return_pcts)
                .bind(&price_cagr_pcts)
                .bind(&total_shares)
                .bind(&total_cashes)
                .bind(&total_end_values)
                .bind(&total_return_pcts)
                .bind(&total_cagr_pcts)
                .bind(&reinv_shares)
                .bind(&reinv_cashes)
                .bind(&reinv_end_values)
                .bind(&reinv_return_pcts)
                .bind(&reinv_cagr_pcts)
                .bind(&first_quote_dates)
                .bind(&shortfall_days)
                .bind(&data_completes)
                .bind(&has_anomalies)
                .bind(&dividend_events)
                .execute(database::get_connection())
                .await
                .context("Failed to upsert stock_cagr in PgCagrRepository::save_batch")?;

            affected += result.rows_affected();
        }

        Ok(affected)
    }

    /// 取得指定基準日與期間的排行榜。
    ///
    /// 除了 `data_complete = true`，也排除該口徑年化報酬率為 `NULL` 的資料列——
    /// 長期間不提供純價格口徑（見 [`CagrPeriod::supports_price_metric`]），
    /// 若不過濾，依 `price_cagr_pct` 排序的結果會混入一批空值列。
    async fn fetch_ranking(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<DomainStockCagr>> {
        // 排序鍵來自白名單映射的字面值，未拼接任何外部輸入；
        // date/period/limit/offset 一律走繫結參數。
        let column = cagr_column(metric);
        let sql = format!(
            r#"
SELECT {SELECT_COLUMNS}
FROM stock_cagr
WHERE date = $1
  AND period = $2
  AND data_complete = true
  AND {column} IS NOT NULL
ORDER BY {column} DESC, stock_symbol
LIMIT $3 OFFSET $4
"#
        );

        let rows = sqlx::query_as::<_, TableStockCagr>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(date)
            .bind(period.code())
            .bind(limit)
            .bind(offset)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch stock_cagr ranking in PgCagrRepository::fetch_ranking")?;

        rows_to_domain(rows)
    }

    /// 依完整查詢條件取得排行榜單頁結果。
    ///
    /// 四個要點：
    ///
    /// 1. 名次由「排序鍵可算」的資料列以 `ROW_NUMBER()` 產生，資料不足者
    ///    不佔名次（`rank` 為 `NULL`）且以 `rankable DESC` 排在所有可算列
    ///    之後——否則排行榜會出現「第 1204 名，報酬率 —」這種無意義的列。
    /// 2. **名次是全市場名次，與篩選條件無關**：`ROW_NUMBER()` 在套用市場／
    ///    產業／關鍵字篩選之前就算完，因此篩出單一產業後可能看到 1、17、43，
    ///    而不是 1、2、3。這讓 `rank` 成為股票的穩定屬性。
    /// 3. 總筆數則相反，是**套用篩選後**的筆數，且另以一次 `COUNT(*)` 取得
    ///    而非取自本頁的視窗欄位：位移超出範圍時本頁是空的，若從資料列取
    ///    `total` 會退化成 0 而讓分頁失效。
    /// 4. 排序欄位一律由 [`sort_column`] 白名單映射為字面值；所有外部輸入
    ///    （日期、期間、市場、產業、關鍵字、分頁）全部走繫結參數。
    async fn fetch_ranking_page(&self, query: &CagrRankingQuery) -> Result<CagrRankingPage> {
        let column = sort_column(query.sort, query.metric);
        let pattern = query.keyword.as_deref().map(keyword_pattern);

        // 可算（rankable）＝ 資料齊全且該排序鍵不為 NULL；長期間的純價格口徑
        // 會出現 data_complete = true 但欄位為 NULL 的列，必須一併視為不可算。
        let rankable = format!("(c.data_complete AND c.{column} IS NOT NULL)");
        let incomplete_filter = format!("  AND ($6::bool OR {rankable})");

        let count_sql = format!("SELECT COUNT(*) {RANKING_FROM_WHERE}\n{incomplete_filter}\n");
        let total: (i64,) = sqlx::query_as(sqlx::AssertSqlSafe(count_sql.as_str()))
            .bind(query.date)
            .bind(query.period.code())
            .bind(query.market_id)
            .bind(query.industry_id)
            .bind(pattern.as_deref())
            .bind(query.include_incomplete)
            .fetch_one(database::get_connection())
            .await
            .context("Failed to count stock_cagr in PgCagrRepository::fetch_ranking_page")?;

        // 名次在 `ranked` CTE（尚未套用任何篩選）就算完：rank 是「該
        // (基準日, 期間, 口徑) 之下全市場的名次」，切換市場／產業／關鍵字
        // 篩選不會改變同一檔股票的名次。若在篩選後重新編號，篩出半導體
        // 後的「第 1 名」其實不是市場第 1 名，顯示成 1 會誤導使用者。
        let sql = format!(
            r#"
WITH ranked AS (
    SELECT {RANKING_SELECT_COLUMNS},
           {rankable} AS rankable,
           CASE
               WHEN {rankable} THEN ROW_NUMBER() OVER (
                   PARTITION BY {rankable}
                   ORDER BY c.{column} DESC, c.stock_symbol
               )
           END AS "rank"
    FROM stock_cagr c
    WHERE c.date = $1
      AND c.period = $2
),
filtered AS (
    SELECT r.*,
           s."Name" AS name,
           s.stock_industry_id AS industry_id
    FROM ranked r
    JOIN stocks s ON s.stock_symbol = r.stock_symbol
    WHERE ($3::int IS NULL OR s.stock_exchange_market_id = $3)
      AND ($4::int IS NULL OR s.stock_industry_id = $4)
      AND ($5::text IS NULL OR r.stock_symbol ILIKE $5 OR s."Name" ILIKE $5)
      AND ($6::bool OR r.rankable)
)
SELECT f.*
FROM filtered f
ORDER BY f.rankable DESC, f."rank" NULLS LAST, f.stock_symbol
LIMIT $7 OFFSET $8
"#
        );

        let rows = sqlx::query_as::<_, RankingRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(query.date)
            .bind(query.period.code())
            .bind(query.market_id)
            .bind(query.industry_id)
            .bind(pattern.as_deref())
            .bind(query.include_incomplete)
            .bind(query.limit)
            .bind(query.offset)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch stock_cagr page in PgCagrRepository::fetch_ranking_page")?;

        let items = rows
            .into_iter()
            .map(|row| {
                Ok(CagrRankingItem {
                    cagr: row.cagr.to_domain()?,
                    name: row.name,
                    industry_id: row.industry_id,
                    rank: row.rank,
                })
            })
            .collect::<Result<Vec<CagrRankingItem>>>()?;

        // 涵蓋統計刻意不套用篩選條件：它回答的是「這個 (基準日, 期間) 的
        // 母體有多少檔算得出來」，是存活者偏誤的揭露依據，不隨畫面篩選變動。
        let coverage = self
            .fetch_coverage(query.date, query.period, query.metric)
            .await?;

        Ok(CagrRankingPage {
            items,
            total: total.0,
            coverage,
        })
    }

    /// 取得單一個股在指定基準日的所有期間結果（含資料不足者）。
    ///
    /// 期間代碼在資料庫是字串，字典序（M3 < M6 < Y1 < Y10 < Y1H …）與時間長度
    /// 不一致，故排序改在領域層以 [`CagrPeriod::months`] 進行。
    async fn fetch_by_symbol(
        &self,
        date: NaiveDate,
        stock_symbol: &str,
    ) -> Result<Vec<DomainStockCagr>> {
        let sql = format!(
            r#"
SELECT {SELECT_COLUMNS}
FROM stock_cagr
WHERE date = $1 AND stock_symbol = $2
"#
        );

        let rows = sqlx::query_as::<_, TableStockCagr>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(date)
            .bind(stock_symbol)
            .fetch_all(database::get_connection())
            .await
            .context("Failed to fetch stock_cagr in PgCagrRepository::fetch_by_symbol")?;

        let mut result = rows_to_domain(rows)?;
        result.sort_by_key(|item| item.period.months());

        Ok(result)
    }

    /// 取得指定基準日與期間的樣本涵蓋統計。
    ///
    /// 五個計數以單一查詢的 `COUNT(*) FILTER` 完成，避免多次往返造成
    /// 各計數取自不同時點的資料而彼此矛盾。
    async fn fetch_coverage(
        &self,
        date: NaiveDate,
        period: CagrPeriod,
        metric: CagrMetric,
    ) -> Result<CagrCoverage> {
        // 同 fetch_ranking：欄位名稱來自白名單字面值。
        let column = cagr_column(metric);
        let sql = format!(
            r#"
SELECT
    COUNT(*) AS universe,
    COUNT(*) FILTER (WHERE data_complete) AS counted,
    COUNT(*) FILTER (WHERE NOT data_complete) AS incomplete,
    COUNT(*) FILTER (WHERE has_anomaly) AS anomaly_flagged,
    COUNT(*) FILTER (WHERE data_complete AND {column} > 0) AS positive
FROM stock_cagr
WHERE date = $1 AND period = $2
"#
        );

        let row = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(date)
            .bind(period.code())
            .fetch_one(database::get_connection())
            .await
            .context("Failed to fetch coverage in PgCagrRepository::fetch_coverage")?;

        Ok(CagrCoverage {
            universe: row.try_get("universe")?,
            counted: row.try_get("counted")?,
            incomplete: row.try_get("incomplete")?,
            anomaly_flagged: row.try_get("anomaly_flagged")?,
            positive: row.try_get("positive")?,
        })
    }

    /// 取得最新一個已完成計算的基準日。
    async fn fetch_latest_date(&self) -> Result<Option<NaiveDate>> {
        let row: (Option<NaiveDate>,) = sqlx::query_as("SELECT MAX(date) FROM stock_cagr")
            .fetch_one(database::get_connection())
            .await
            .context("Failed to fetch latest date in PgCagrRepository::fetch_latest_date")?;

        Ok(row.0)
    }

    /// 刪除早於指定日期的歷史資料。
    async fn delete_before(&self, date: NaiveDate) -> Result<u64> {
        let result = sqlx::query("DELETE FROM stock_cagr WHERE date < $1")
            .bind(date)
            .execute(database::get_connection())
            .await
            .context("Failed to delete stock_cagr in PgCagrRepository::delete_before")?;

        Ok(result.rows_affected())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::performance::entity::SimulationOutcome;
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    /// 測試專用的假代號前綴；真實市場不存在此代號。
    const FAKE_SYMBOL: &str = "79979";
    /// 排行榜測試用的另外兩個假代號。
    const FAKE_SYMBOL_B: &str = "79979B";
    const FAKE_SYMBOL_C: &str = "79979C";
    const FAKE_SYMBOL_D: &str = "79979D";

    /// 測試基準日。
    ///
    /// 刻意選 1990-01-02 這種遠早於本功能上線的日期：`fetch_coverage`／
    /// `fetch_ranking` 是以 (date, period) 為範圍的整體統計，若沿用近期日期，
    /// 正式資料會混進計數而讓斷言時好時壞。
    fn test_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(1990, 1, 2).unwrap()
    }

    /// 建立一筆資料齊全的測試資料。
    fn complete_record(symbol: &str, period: CagrPeriod, cagr: Decimal) -> DomainStockCagr {
        let outcome = SimulationOutcome {
            end_shares: dec!(100.5),
            cash_received: dec!(250.0),
            end_value: dec!(12000.0),
            total_return_pct: dec!(20.0),
            cagr_pct: cagr,
        };

        DomainStockCagr {
            date: test_date(),
            stock_symbol: symbol.to_string(),
            period,
            base_date: NaiveDate::from_ymd_opt(1989, 1, 3),
            base_price: Some(dec!(100.0)),
            end_price: Some(dec!(119.4)),
            years: Some(dec!(1.0)),
            price: Some(SimulationOutcome {
                cash_received: Decimal::ZERO,
                cagr_pct: cagr - dec!(1),
                ..outcome
            }),
            total: Some(outcome),
            reinvested: Some(SimulationOutcome {
                cash_received: Decimal::ZERO,
                cagr_pct: cagr + dec!(1),
                ..outcome
            }),
            first_quote_date: NaiveDate::from_ymd_opt(1988, 1, 4),
            shortfall_days: Some(0),
            data_complete: true,
            has_anomaly: false,
            dividend_events: 2,
        }
    }

    /// 建立一筆資料不足的測試資料（所有數值欄位皆為 None）。
    fn incomplete_record(symbol: &str, period: CagrPeriod) -> DomainStockCagr {
        DomainStockCagr {
            date: test_date(),
            stock_symbol: symbol.to_string(),
            period,
            base_date: None,
            base_price: None,
            end_price: None,
            years: None,
            price: None,
            total: None,
            reinvested: None,
            first_quote_date: NaiveDate::from_ymd_opt(1989, 6, 1),
            shortfall_days: None,
            data_complete: false,
            has_anomaly: true,
            dividend_events: 0,
        }
    }

    /// 清理測試寫入的資料列；測試結束務必呼叫，避免污染資料庫。
    async fn cleanup() {
        let _ = sqlx::query("DELETE FROM stock_cagr WHERE stock_symbol = ANY($1)")
            .bind(vec![
                FAKE_SYMBOL.to_string(),
                FAKE_SYMBOL_B.to_string(),
                FAKE_SYMBOL_C.to_string(),
                FAKE_SYMBOL_D.to_string(),
            ])
            .execute(database::get_connection())
            .await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_save_batch_round_trip_and_idempotent() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_save_batch_round_trip_and_idempotent：無資料庫連接");
            return;
        }

        let repo = PgCagrRepository::new();
        cleanup().await;

        // 1. 寫入後讀回應一致
        let record = complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(12.3456));
        let affected = repo
            .save_batch(std::slice::from_ref(&record))
            .await
            .expect("save_batch");
        assert_eq!(affected, 1);

        let fetched = repo
            .fetch_by_symbol(test_date(), FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol");
        assert_eq!(fetched.len(), 1);
        let first = &fetched[0];
        assert_eq!(first.stock_symbol, record.stock_symbol);
        assert_eq!(first.period, CagrPeriod::Y1);
        assert_eq!(first.base_date, record.base_date);
        assert_eq!(first.total, record.total);
        assert_eq!(first.reinvested, record.reinvested);
        assert!(first.data_complete);
        assert_eq!(first.dividend_events, 2);

        // 2. 同一主鍵重複寫入是覆蓋而非新增
        let mut updated = complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(99.9999));
        updated.dividend_events = 7;
        repo.save_batch(&[updated]).await.expect("save_batch again");

        let fetched = repo
            .fetch_by_symbol(test_date(), FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol");
        assert_eq!(fetched.len(), 1, "重複 upsert 不應新增資料列");
        assert_eq!(fetched[0].total.map(|o| o.cagr_pct), Some(dec!(99.9999)));
        assert_eq!(fetched[0].dividend_events, 7);

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_incomplete_record_reads_back_as_none() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_incomplete_record_reads_back_as_none：無資料庫連接");
            return;
        }

        let repo = PgCagrRepository::new();
        cleanup().await;

        repo.save_batch(&[incomplete_record(FAKE_SYMBOL, CagrPeriod::Y10)])
            .await
            .expect("save_batch");

        let fetched = repo
            .fetch_by_symbol(test_date(), FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol");
        assert_eq!(fetched.len(), 1);
        let item = &fetched[0];
        assert!(!item.data_complete);
        assert!(item.base_price.is_none());
        assert!(item.end_price.is_none());
        assert!(item.years.is_none());
        assert!(item.price.is_none());
        assert!(item.total.is_none(), "資料不足時不可讀成 0，必須是 None");
        assert!(item.reinvested.is_none());
        assert!(item.shortfall_days.is_none());
        assert!(item.has_anomaly);

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch_by_symbol_sorted_by_period_length() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_fetch_by_symbol_sorted_by_period_length：無資料庫連接");
            return;
        }

        let repo = PgCagrRepository::new();
        cleanup().await;

        // 刻意以「字典序會排錯」的組合驗證：Y10 字典序在 Y1H 之前。
        let records = vec![
            complete_record(FAKE_SYMBOL, CagrPeriod::Y10, dec!(1)),
            complete_record(FAKE_SYMBOL, CagrPeriod::M3, dec!(2)),
            incomplete_record(FAKE_SYMBOL, CagrPeriod::Y1H),
            complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(3)),
        ];
        repo.save_batch(&records).await.expect("save_batch");

        let fetched = repo
            .fetch_by_symbol(test_date(), FAKE_SYMBOL)
            .await
            .expect("fetch_by_symbol");
        let periods: Vec<CagrPeriod> = fetched.iter().map(|item| item.period).collect();
        assert_eq!(
            periods,
            vec![
                CagrPeriod::M3,
                CagrPeriod::Y1,
                CagrPeriod::Y1H,
                CagrPeriod::Y10
            ],
            "應依期間長度排序，且包含 data_complete = false 的列"
        );

        cleanup().await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch_ranking_and_coverage() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_fetch_ranking_and_coverage：無資料庫連接");
            return;
        }

        let repo = PgCagrRepository::new();
        cleanup().await;

        // 三筆可算（年化 5 / 15 / -3）＋ 一筆資料不足（同時標記異常）。
        let records = vec![
            complete_record(FAKE_SYMBOL, CagrPeriod::Y1, dec!(5)),
            complete_record(FAKE_SYMBOL_B, CagrPeriod::Y1, dec!(15)),
            complete_record(FAKE_SYMBOL_C, CagrPeriod::Y1, dec!(-3)),
        ];
        repo.save_batch(&records).await.expect("save_batch");
        // 再寫入資料不足的列：Y1 一筆（驗證排行榜排除它）、Y2 兩筆（驗證涵蓋率計數）。
        let incompletes = vec![
            incomplete_record(FAKE_SYMBOL_D, CagrPeriod::Y1),
            incomplete_record(FAKE_SYMBOL, CagrPeriod::Y2),
            incomplete_record(FAKE_SYMBOL_C, CagrPeriod::Y2),
        ];
        repo.save_batch(&incompletes).await.expect("save_batch");

        // 排行榜：依 total 口徑由高至低，且不含資料不足者。
        let ranking = repo
            .fetch_ranking(test_date(), CagrPeriod::Y1, CagrMetric::Total, 10, 0)
            .await
            .expect("fetch_ranking");
        let symbols: Vec<&str> = ranking
            .iter()
            .map(|item| item.stock_symbol.as_str())
            .collect();
        assert_eq!(
            symbols,
            vec![FAKE_SYMBOL_B, FAKE_SYMBOL, FAKE_SYMBOL_C],
            "應依年化報酬率由高至低，且不含 data_complete = false 的 {FAKE_SYMBOL_D}"
        );
        assert!(ranking.iter().all(|item| item.data_complete));

        // offset/limit 亦應生效。
        let paged = repo
            .fetch_ranking(test_date(), CagrPeriod::Y1, CagrMetric::Total, 1, 1)
            .await
            .expect("fetch_ranking paged");
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].stock_symbol, FAKE_SYMBOL);

        // 涵蓋率統計：Y1 期間共 4 列，3 列可算（正報酬 2 檔），1 列資料不足且標記異常。
        let coverage = repo
            .fetch_coverage(test_date(), CagrPeriod::Y1, CagrMetric::Total)
            .await
            .expect("fetch_coverage");
        assert_eq!(coverage.universe, 4);
        assert_eq!(coverage.counted, 3);
        assert_eq!(coverage.incomplete, 1);
        assert_eq!(coverage.anomaly_flagged, 1);
        assert_eq!(coverage.positive, 2);

        // Y2 期間共 2 列，皆為資料不足且標記異常。
        let coverage = repo
            .fetch_coverage(test_date(), CagrPeriod::Y2, CagrMetric::Total)
            .await
            .expect("fetch_coverage");
        assert_eq!(coverage.universe, 2);
        assert_eq!(coverage.counted, 0);
        assert_eq!(coverage.incomplete, 2);
        assert_eq!(coverage.anomaly_flagged, 2);
        assert_eq!(coverage.positive, 0);

        // 最新基準日必定不早於測試用的 1990-01-02。
        let latest = repo.fetch_latest_date().await.expect("fetch_latest_date");
        assert!(latest.is_some_and(|d| d >= test_date()));

        cleanup().await;
    }

    /// 排行榜分頁查詢：全市場名次、資料不足殿後、篩選與分頁語意。
    ///
    /// 名次必須是「未套用篩選的全市場名次」，因此以產業／關鍵字篩選後，
    /// 同一檔股票的 `rank` 不可改變——這是前端把 rank 當成股票穩定屬性的
    /// 前提。測試借用 `stocks` 既有的真實代號（唯讀取得）以滿足 JOIN，
    /// 並只在遠早於本功能上線的 1990-01-02 寫入，結束後一律清除。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch_ranking_page_semantics() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_fetch_ranking_page_semantics：無資料庫連接");
            return;
        }

        // 排行榜要 JOIN stocks，因此借用真實存在的代號；同產業取三檔以便
        // 驗證「篩選後名次不重編」。
        let borrowed: Vec<(String, i32)> = sqlx::query_as(
            r#"SELECT stock_symbol, stock_industry_id FROM stocks
               WHERE stock_industry_id = (
                   SELECT stock_industry_id FROM stocks
                   GROUP BY stock_industry_id HAVING COUNT(*) >= 4 LIMIT 1
               )
               ORDER BY stock_symbol LIMIT 4"#,
        )
        .fetch_all(database::get_connection())
        .await
        .expect("borrow stocks");
        if borrowed.len() < 4 {
            println!("跳過 test_fetch_ranking_page_semantics：stocks 樣本不足");
            return;
        }
        let symbols: Vec<String> = borrowed.iter().map(|(symbol, _)| symbol.clone()).collect();
        let industry = borrowed[0].1;
        let other_industry: Option<i32> = sqlx::query_scalar(
            "SELECT stock_industry_id FROM stocks WHERE stock_industry_id <> $1 LIMIT 1",
        )
        .bind(industry)
        .fetch_optional(database::get_connection())
        .await
        .expect("other industry");

        let cleanup_borrowed = || async {
            let _ =
                sqlx::query("DELETE FROM stock_cagr WHERE date = $1 AND stock_symbol = ANY($2)")
                    .bind(test_date())
                    .bind(&symbols)
                    .execute(database::get_connection())
                    .await;
        };
        cleanup_borrowed().await;

        let repo = PgCagrRepository::new();
        // 年化 30 / 20 / 10 三檔可算，第四檔資料不足。
        let records = vec![
            complete_record(&symbols[0], CagrPeriod::Y1, dec!(30)),
            complete_record(&symbols[1], CagrPeriod::Y1, dec!(20)),
            complete_record(&symbols[2], CagrPeriod::Y1, dec!(10)),
            incomplete_record(&symbols[3], CagrPeriod::Y1),
        ];
        repo.save_batch(&records).await.expect("save_batch");

        let base = CagrRankingQuery::new(test_date(), CagrPeriod::Y1);
        let page = repo
            .fetch_ranking_page(&base)
            .await
            .expect("fetch_ranking_page");
        let listed: Vec<(&str, Option<i64>)> = page
            .items
            .iter()
            .filter(|item| symbols.contains(&item.cagr.stock_symbol))
            .map(|item| (item.cagr.stock_symbol.as_str(), item.rank))
            .collect();
        assert_eq!(listed.len(), 4, "資料不足者也必須出現在清單中");
        // 可算三檔依年化由高至低，且名次遞增；資料不足者 rank 為 None。
        assert_eq!(listed[0].0, symbols[0]);
        assert_eq!(listed[1].0, symbols[1]);
        assert_eq!(listed[2].0, symbols[2]);
        assert_eq!(listed[3].0, symbols[3]);
        assert!(listed[3].1.is_none(), "資料不足不得佔名次");
        let ranks: Vec<i64> = listed[..3].iter().filter_map(|(_, rank)| *rank).collect();
        assert_eq!(ranks.len(), 3);
        assert!(ranks[0] < ranks[1] && ranks[1] < ranks[2]);
        assert_eq!(page.items.len() as i64, page.total.min(base.limit));
        assert!(page.items.iter().all(|item| !item.name.is_empty()));

        // 產業篩選：名次不得重編，仍是全市場名次。
        let filtered = repo
            .fetch_ranking_page(&CagrRankingQuery {
                industry_id: Some(industry),
                ..base.clone()
            })
            .await
            .expect("fetch_ranking_page industry");
        let filtered_ranks: Vec<Option<i64>> = filtered
            .items
            .iter()
            .filter(|item| symbols.contains(&item.cagr.stock_symbol))
            .map(|item| item.rank)
            .collect();
        assert_eq!(
            filtered_ranks,
            listed.iter().map(|(_, rank)| *rank).collect::<Vec<_>>(),
            "套用篩選後名次必須維持全市場名次"
        );
        assert!(filtered.total <= page.total);
        assert_eq!(filtered.coverage, page.coverage, "涵蓋統計不隨畫面篩選變動");

        // 其他產業必然不含這四檔。
        if let Some(other) = other_industry {
            let elsewhere = repo
                .fetch_ranking_page(&CagrRankingQuery {
                    industry_id: Some(other),
                    ..base.clone()
                })
                .await
                .expect("fetch_ranking_page other industry");
            assert!(
                elsewhere
                    .items
                    .iter()
                    .all(|item| !symbols.contains(&item.cagr.stock_symbol))
            );
        }

        // include_incomplete = false 時資料不足者整列消失，total 同步變小。
        let complete_only = repo
            .fetch_ranking_page(&CagrRankingQuery {
                include_incomplete: false,
                ..base.clone()
            })
            .await
            .expect("fetch_ranking_page complete only");
        assert!(
            complete_only
                .items
                .iter()
                .all(|item| item.rank.is_some() && item.cagr.data_complete)
        );
        assert!(complete_only.total < page.total);

        // 關鍵字比對代號；分頁位移超出範圍時 total 仍須正確。
        let keyword = repo
            .fetch_ranking_page(&CagrRankingQuery {
                keyword: Some(symbols[0].clone()),
                ..base.clone()
            })
            .await
            .expect("fetch_ranking_page keyword");
        assert!(
            keyword
                .items
                .iter()
                .any(|item| item.cagr.stock_symbol == symbols[0])
        );
        let beyond = repo
            .fetch_ranking_page(&CagrRankingQuery {
                offset: page.total + 100,
                ..base.clone()
            })
            .await
            .expect("fetch_ranking_page beyond");
        assert!(beyond.items.is_empty());
        assert_eq!(beyond.total, page.total, "位移超界不可讓 total 退化成 0");

        cleanup_borrowed().await;
    }
}
