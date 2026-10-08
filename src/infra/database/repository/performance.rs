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

    async fn fetch_dates_missing_period(&self, period: CagrPeriod) -> Result<Vec<NaiveDate>> {
        // 以 NOT EXISTS 而非 LEFT JOIN：只要該日期出現過任何一列指定期間的
        // 結果就算已回填，不需要逐檔比對母體。
        let sql = r#"
            SELECT DISTINCT s.date
            FROM stock_cagr s
            WHERE NOT EXISTS (
                SELECT 1
                FROM stock_cagr t
                WHERE t.date = s.date AND t.period = $1
            )
            ORDER BY s.date
        "#;

        let rows: Vec<(NaiveDate,)> = sqlx::query_as(sql)
            .bind(period.code())
            .fetch_all(database::get_connection())
            .await
            .context(
                "Failed to fetch dates missing period in PgCagrRepository::fetch_dates_missing_period",
            )?;

        Ok(rows.into_iter().map(|(date,)| date).collect())
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
mod tests;
