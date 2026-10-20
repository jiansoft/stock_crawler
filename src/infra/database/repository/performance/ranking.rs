//! CAGR 倉儲的排行榜查詢：單一口徑排行與含篩選、名次、分頁的排行頁。

use anyhow::{Context, Result};
use chrono::NaiveDate;

use super::{SELECT_COLUMNS, cagr_column, lookup, rows_to_domain};
use crate::domain::performance::entity::{CagrMetric, CagrPeriod, StockCagr as DomainStockCagr};
use crate::domain::performance::query::{
    CagrRankingItem, CagrRankingPage, CagrRankingQuery, CagrSortKey,
};
use crate::infra::database;
use crate::infra::database::table::performance::stock_cagr::StockCagr as TableStockCagr;

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

/// 取得指定基準日與期間的排行榜。
///
/// 除了 `data_complete = true`，也排除該口徑年化報酬率為 `NULL` 的資料列——
/// 長期間不提供純價格口徑（見 [`CagrPeriod::supports_price_metric`]），
/// 若不過濾，依 `price_cagr_pct` 排序的結果會混入一批空值列。
pub(super) async fn fetch_ranking(
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
pub(super) async fn fetch_ranking_page(query: &CagrRankingQuery) -> Result<CagrRankingPage> {
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
    let coverage = lookup::fetch_coverage(query.date, query.period, query.metric).await?;

    Ok(CagrRankingPage {
        items,
        total: total.0,
        coverage,
    })
}
