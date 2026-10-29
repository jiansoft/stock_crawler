//! 收盤後排行：以 `"DailyQuotes"` 最新交易日計算（非交易時段路徑）。

use super::*;

/// 以 `"DailyQuotes"` 最新一個交易日計算排行（非交易時段路徑）。
///
/// 先取最新交易日再查該日資料，而不是一條 SQL 內嵌子查詢：一來 `data_as_of`
/// 需要這個日期（即使排行為空也要回），二來把日期綁成參數可讓 planner 直接
/// 用 `"DailyQuotes_Date_include_symbol_idx"` 定位單日資料。
pub(super) async fn closing_movers(
    rank_by: MoversRankBy,
    market: &str,
    market_id: i32,
    limit: u8,
) -> Response {
    let latest: Result<Option<NaiveDate>, _> =
        sqlx::query_scalar(r#"SELECT MAX("Date") FROM "DailyQuotes""#)
            .fetch_one(database::get_connection())
            .await;
    let latest_date = match latest {
        Ok(Some(date)) => date,
        // 整張日線表都沒有資料才算「查無排行」；這是部署異常等級的狀況。
        Ok(None) => {
            return error_response(StatusCode::NOT_FOUND, "查無漲跌幅排行資料");
        }
        Err(error) => return database_error(error),
    };

    // ORDER BY 只可能來自 `MoversRankBy::closing_order_by` 的三個固定字面值，
    // 其餘條件全部走 bind parameter。SQLx 0.9 對動態組成的 SQL 要求顯式稽核
    // （AssertSqlSafe），做法與 screen_stocks／qfii_holding_ranking 一致。
    let sql = format!(
        "{CLOSING_MOVERS_SQL}\n{}\nLIMIT $3",
        rank_by.closing_order_by()
    );
    let rows: Result<Vec<ClosingMoverRow>, _> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(latest_date)
        .bind(market_id)
        .bind(i64::from(limit))
        .fetch_all(database::get_connection())
        .await;
    match rows {
        Ok(rows) => Json(MarketMoversResponse {
            data_as_of: latest_date.format("%Y-%m-%d").to_string(),
            source: "closing".to_owned(),
            is_realtime: false,
            rank_by: rank_by.as_str().to_owned(),
            market: market.to_owned(),
            // 收盤來源沒有「快照時間」的概念，依 §3.1 不可偽造一個。
            snapshot_updated_at: None,
            movers: rows
                .into_iter()
                .enumerate()
                .map(|(index, row)| row.into_dto(index as u32 + 1))
                .collect(),
        })
        .into_response(),
        Err(error) => database_error(error),
    }
}

/// 收盤來源排行的參數化 SQL 主體（不含排序與 LIMIT）。
///
/// 綁定參數：
/// - `$1`：交易日（由呼叫端先查出的最新一日）。
/// - `$2`：市場 id；`0` 是 API 內部的 all 哨兵，只展開為固定 `IN (2, 4)`。
/// - `$3`：回傳筆數上限（附加於排序分支之後）。
///
/// 過濾條件寫死在 SQL：暫停上市股票不應出現在排行；`"TradingVolume" > 0`
/// 排除當日無成交的股票——沒有成交的漲跌幅與成交量排名都沒有意義。
///
/// 資料型別備註：`"DailyQuotes"` 的價量欄位皆為 `numeric(18,4) NOT NULL`，
/// 因此 SQL 端不會出現 NULL；DTO 仍用 `Option<f64>`，是為了在 `NUMERIC` 無法
/// 安全轉成 `f64` 時可以依 §3.1 輸出 `null` 而非錯誤的數字。
pub(super) const CLOSING_MOVERS_SQL: &str = r#"
SELECT q.stock_symbol,
       s."Name" AS name,
       s.stock_exchange_market_id AS market_id,
       s.stock_industry_id AS industry_id,
       q."ClosingPrice" AS price,
       q."Change" AS change,
       q."ChangeRange" AS change_percent,
       q."OpeningPrice" AS open_price,
       q."HighestPrice" AS high_price,
       q."LowestPrice" AS low_price,
       q."TradingVolume" AS volume_shares,
       q."TradeValue" AS trade_value,
       q."Transaction" AS transaction_count
FROM "DailyQuotes" q
JOIN stocks s ON s.stock_symbol = q.stock_symbol
WHERE q."Date" = $1
  AND (($2 = 0 AND s.stock_exchange_market_id IN (2, 4))
    OR ($2 <> 0 AND s.stock_exchange_market_id = $2))
  AND s."SuspendListing" = false
  AND q."TradingVolume" > 0
"#;

/// 收盤來源排行的資料庫列。
#[derive(sqlx::FromRow)]
pub(super) struct ClosingMoverRow {
    /// 股票代號。
    pub(super) stock_symbol: String,
    /// 股票名稱。
    pub(super) name: String,
    /// 市場 id（上市 2、上櫃 4）。
    pub(super) market_id: i32,
    /// 產業分類 id。
    pub(super) industry_id: i32,
    /// 收盤價。
    pub(super) price: Decimal,
    /// 漲跌（元）。
    pub(super) change: Decimal,
    /// 漲跌幅（%）。
    pub(super) change_percent: Decimal,
    /// 開盤價。
    pub(super) open_price: Decimal,
    /// 最高價。
    pub(super) high_price: Decimal,
    /// 最低價。
    pub(super) low_price: Decimal,
    /// 成交股數。
    pub(super) volume_shares: Decimal,
    /// 成交金額（元）。
    pub(super) trade_value: Decimal,
    /// 成交筆數。
    pub(super) transaction_count: Decimal,
}

impl ClosingMoverRow {
    /// 加入查詢結果順序產生的一起始名次並轉成排行 DTO。
    pub(super) fn into_dto(self, rank: u32) -> MarketMover {
        let symbol = self.stock_symbol.clone();
        let convert = |value: Decimal, field: &'static str| {
            analytical_decimal_to_f64(Some(value), &symbol, field)
        };
        MarketMover {
            rank,
            stock_symbol: self.stock_symbol.clone(),
            name: self.name,
            market_id: self.market_id,
            industry_id: self.industry_id,
            price: convert(self.price, "price"),
            change: convert(self.change, "change"),
            change_percent: convert(self.change_percent, "change_percent"),
            open: convert(self.open_price, "open"),
            high: convert(self.high_price, "high"),
            low: convert(self.low_price, "low"),
            // `"DailyQuotes"` 沒有昨收欄位；成交量單位是「股」而非「張」，
            // 兩者依 §4.4 各自輸出 null，不做換算也不冒充。
            last_close: None,
            volume_lots: None,
            volume_shares: convert(self.volume_shares, "volume_shares"),
            trade_value: convert(self.trade_value, "trade_value"),
            transaction: convert(self.transaction_count, "transaction"),
            source_site: None,
        }
    }
}
