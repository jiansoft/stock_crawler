//! `/market/movers` 當日漲跌幅／成交量排行 handler。
//!
//! 同一個 endpoint 依即時快取狀態切換資料來源：盤中走記憶體即時報價快照，
//! 非交易時段走 `"DailyQuotes"` 最新交易日（movers 計畫 §3）。

use std::collections::HashMap;

use axum::{
    Json,
    extract::Query,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use chrono::{Local, NaiveDate};
use rust_decimal::Decimal;

use super::{analytical_decimal_to_f64, database_error, error_response, market_id_for_stocks};
use crate::domain::registry::entity::Stock;
use crate::infra::{
    cache::{RealtimeSnapshot, SHARE},
    database,
};
use crate::interfaces::web::data_api::dto::{
    ErrorBody, MarketMover, MarketMoversParams, MarketMoversResponse,
};

/// 排行的排序鍵（movers 計畫 §4.1）。
///
/// 只在 handler 內部使用；把呼叫端傳來的字串**先轉成這個 enum**，之後所有
/// 分支都對 enum 做比對，SQL 片段也只能從固定的 `&'static str` 取得，呼叫端
/// 沒有任何機會把字串帶進 SQL。
// 變體名稱刻意與對外的 `rank_by` 字面值一一對應（`TopGainers` ↔ `top_gainers`），
// 讓 handler 讀起來就是契約本身；共同前綴是這個對應關係的結果，不是命名疏漏。
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MoversRankBy {
    /// 漲幅由高到低。
    TopGainers,
    /// 漲跌幅由低到高（跌最深的在前）。
    TopLosers,
    /// 成交量由大到小。
    TopVolume,
}

impl MoversRankBy {
    /// 將 query string 轉成排序鍵；未知值回 `None`（由呼叫端回 422）。
    fn parse(value: &str) -> Option<Self> {
        match value {
            "top_gainers" => Some(Self::TopGainers),
            "top_losers" => Some(Self::TopLosers),
            "top_volume" => Some(Self::TopVolume),
            _ => None,
        }
    }

    /// 回寫進 response 的字面值，讓呼叫端確認伺服器實際採用的排序鍵。
    fn as_str(self) -> &'static str {
        match self {
            Self::TopGainers => "top_gainers",
            Self::TopLosers => "top_losers",
            Self::TopVolume => "top_volume",
        }
    }

    /// 收盤來源要用的 `ORDER BY` 片段（固定字面值，不含任何呼叫端輸入）。
    ///
    /// 一律以 `stock_symbol ASC` 作為同值時的第二排序鍵，確保同分股票的名次
    /// 在多次查詢間穩定，不會因 PostgreSQL 的掃描順序而跳動。
    fn closing_order_by(self) -> &'static str {
        match self {
            Self::TopGainers => r#"ORDER BY q."ChangeRange" DESC, q.stock_symbol ASC"#,
            Self::TopLosers => r#"ORDER BY q."ChangeRange" ASC, q.stock_symbol ASC"#,
            Self::TopVolume => r#"ORDER BY q."TradingVolume" DESC, q.stock_symbol ASC"#,
        }
    }
}

/// 查詢當日漲跌幅／成交量排行（movers 計畫 §4）。
///
/// # 為什麼一個 endpoint 要接兩種資料來源？
///
/// 台股「今天漲最多的是哪幾檔」在盤中與收盤後是同一個問題，但資料放在兩個
/// 地方：盤中的即時報價在記憶體快取（由 HiStock／Yahoo 背景任務寫入），當日
/// 最終結果則在 15:00 收盤排程寫進 `"DailyQuotes"`。若拆成兩個 endpoint，
/// 呼叫端（通常是 LLM）就得自己判斷「現在是不是盤中」——它沒有這個能力，
/// 判斷錯就會把前一交易日的收盤資料當成今日行情回答使用者。因此來源切換
/// 一律由伺服器端決定，並把結果誠實寫在 `source`／`is_realtime`／`data_as_of`
/// 三個欄位裡。
///
/// 切換規則（movers 計畫 §3）：
/// - 即時快取非空 → `realtime`，純記憶體計算，不碰資料庫。
/// - 即時快取為空 → `closing`，查 `"DailyQuotes"` 最新一個交易日。
/// - 兩者都沒有資料 → 404。
///
/// 判定依據是「採集任務有沒有在跑」（快取是否為空），而不是看時鐘：服務
/// 重啟、國定假日、颱風停市等情況下時鐘會判斷錯，快取狀態不會。
///
/// # Errors
///
/// `rank_by`、`market` 不在固定 enum 或 `limit` 超出 1–50 回 422；驗證失敗回
/// 401；完全沒有可用資料回 404；資料庫查詢失敗時記錄內部錯誤並回不含 SQL
/// 細節的 500。
#[utoipa::path(get, path = "/api/v1/market/movers", tag = "data-api", params(MarketMoversParams), responses((status = 200, body = MarketMoversResponse), (status = 401, body = ErrorBody), (status = 404, body = ErrorBody), (status = 422, body = ErrorBody), (status = 500, body = ErrorBody)), security(("bearer_auth" = [])))]
pub(crate) async fn market_movers(Query(params): Query<MarketMoversParams>) -> Response {
    // 先驗證所有參數再取資料：格式錯誤是呼叫端的問題（422），不該消耗
    // 記憶體掃描或資料庫資源。
    let Some(rank_by) = MoversRankBy::parse(params.rank_by.as_deref().unwrap_or("top_gainers"))
    else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "rank_by 必須為 top_gainers、top_losers 或 top_volume",
        );
    };
    let market = params.market.as_deref().unwrap_or("all");
    let Some(market_id) = market_id_for_stocks(market) else {
        return error_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            "market 必須為 all、twse 或 tpex",
        );
    };
    let limit = params.limit.unwrap_or(20);
    if !(1..=50).contains(&limit) {
        return error_response(StatusCode::UNPROCESSABLE_ENTITY, "limit 必須介於 1 至 50");
    }

    // 資料來源切換的唯一判斷點：快取非空代表採集任務正在跑，也就是盤中。
    if !SHARE.stock_snapshots_are_empty() {
        return Json(realtime_movers(rank_by, market, market_id, limit)).into_response();
    }
    closing_movers(rank_by, market, market_id, limit).await
}

/// 即時排行的候選股票：快照加上從股票主檔對照到的名稱、市場與產業。
struct RealtimeCandidate {
    /// 即時報價快照。
    snapshot: RealtimeSnapshot,
    /// 股票名稱；快照沒有名稱時取主檔名稱。
    name: String,
    /// 市場 id（上市 2、上櫃 4）。
    market_id: i32,
    /// 產業分類 id。
    industry_id: i32,
}

/// 以記憶體中的即時報價快照計算排行（盤中路徑）。
///
/// 快照本身只有代號、名稱與報價，沒有市場別與產業別，因此必須跟
/// [`SHARE`] 的股票主檔快取對照後才能套用 `market` 條件。主檔快取只鎖一次、
/// 在鎖內只做對照與複製，離開鎖後才排序，避免拖慢盤中高頻寫入快照的採集任務。
fn realtime_movers(
    rank_by: MoversRankBy,
    market: &str,
    market_id: i32,
    limit: u8,
) -> MarketMoversResponse {
    let snapshots = SHARE.all_stock_snapshots();
    let (candidates, unknown_symbols) = match SHARE.stocks.read() {
        Ok(stocks) => select_realtime_candidates(snapshots, &stocks, market_id),
        Err(error) => {
            // 主檔快取讀鎖毒化時無法判斷市場別，寧可回空排行也不輸出未經
            // 市場條件過濾的資料。
            tracing::error!(?error, "股票主檔快取讀取失敗，即時排行改回空清單");
            (Vec::new(), Vec::new())
        }
    };
    if !unknown_symbols.is_empty() {
        tracing::warn!(
            count = unknown_symbols.len(),
            symbols = ?unknown_symbols.iter().take(10).collect::<Vec<_>>(),
            "即時快照中的代號在股票主檔找不到，已排除於排行之外"
        );
    }
    rank_realtime_candidates(candidates, rank_by, market, limit)
}

/// 將即時快照與股票主檔對照並套用過濾條件，回傳候選股票與主檔查不到的代號。
///
/// 過濾條件與收盤來源保持一致（movers 計畫 §4.5）：暫停上市、市場別不符、
/// 零成交量者都不進排行。主檔查不到的代號（例如剛上市尚未同步）一律排除，
/// 不猜測它屬於哪個市場。
fn select_realtime_candidates(
    snapshots: Vec<RealtimeSnapshot>,
    stocks: &HashMap<String, Stock>,
    market_id: i32,
) -> (Vec<RealtimeCandidate>, Vec<String>) {
    let mut candidates = Vec::new();
    let mut unknown_symbols = Vec::new();
    for snapshot in snapshots {
        let Some(stock) = stocks.get(&snapshot.symbol) else {
            unknown_symbols.push(snapshot.symbol);
            continue;
        };
        if stock.suspend_listing()
            || !market_matches(market_id, stock.market_id())
            || snapshot.volume <= Decimal::ZERO
        {
            continue;
        }
        let name = if snapshot.name.is_empty() {
            stock.name().to_owned()
        } else {
            snapshot.name.clone()
        };
        candidates.push(RealtimeCandidate {
            snapshot,
            name,
            market_id: stock.market_id(),
            industry_id: stock.industry_id(),
        });
    }
    (candidates, unknown_symbols)
}

/// 依排序鍵排出即時排行並轉成回應。
///
/// 排序鍵一律搭配股票代號作為第二鍵，讓同值股票的名次穩定。
fn rank_realtime_candidates(
    mut candidates: Vec<RealtimeCandidate>,
    rank_by: MoversRankBy,
    market: &str,
    limit: u8,
) -> MarketMoversResponse {
    match rank_by {
        MoversRankBy::TopGainers => candidates.sort_by(|left, right| {
            right
                .snapshot
                .change_range
                .cmp(&left.snapshot.change_range)
                .then_with(|| left.snapshot.symbol.cmp(&right.snapshot.symbol))
        }),
        MoversRankBy::TopLosers => candidates.sort_by(|left, right| {
            left.snapshot
                .change_range
                .cmp(&right.snapshot.change_range)
                .then_with(|| left.snapshot.symbol.cmp(&right.snapshot.symbol))
        }),
        MoversRankBy::TopVolume => candidates.sort_by(|left, right| {
            right
                .snapshot
                .volume
                .cmp(&left.snapshot.volume)
                .then_with(|| left.snapshot.symbol.cmp(&right.snapshot.symbol))
        }),
    }

    // 快照批次的最新更新時間取全體最大值，讓呼叫端能判斷資料新鮮度。
    let snapshot_updated_at = candidates
        .iter()
        .map(|candidate| candidate.snapshot.updated_at)
        .max()
        .map(|updated_at| updated_at.to_rfc3339());
    candidates.truncate(usize::from(limit));

    MarketMoversResponse {
        // 盤中即時資料的日期就是台北時區的今天（容器時區為 Asia/Taipei）。
        data_as_of: Local::now().date_naive().format("%Y-%m-%d").to_string(),
        source: "realtime".to_owned(),
        is_realtime: true,
        rank_by: rank_by.as_str().to_owned(),
        market: market.to_owned(),
        snapshot_updated_at,
        movers: candidates
            .into_iter()
            .enumerate()
            .map(|(index, candidate)| realtime_mover(index as u32 + 1, candidate))
            .collect(),
    }
}

/// 將即時候選股票轉成排行 DTO。
fn realtime_mover(rank: u32, candidate: RealtimeCandidate) -> MarketMover {
    let RealtimeCandidate {
        snapshot,
        name,
        market_id,
        industry_id,
    } = candidate;
    let convert = |value: Decimal, field: &'static str| {
        analytical_decimal_to_f64(Some(value), &snapshot.symbol, field)
    };
    MarketMover {
        rank,
        stock_symbol: snapshot.symbol.clone(),
        name,
        market_id,
        industry_id,
        price: convert(snapshot.price, "price"),
        change: convert(snapshot.change, "change"),
        change_percent: convert(snapshot.change_range, "change_percent"),
        open: convert(snapshot.open, "open"),
        high: convert(snapshot.high, "high"),
        low: convert(snapshot.low, "low"),
        last_close: convert(snapshot.last_close, "last_close"),
        // 即時快照的成交量單位是「張」，成交金額與成交筆數則完全沒有，
        // 依 §3.1 一律輸出 null，不可用 0 冒充。
        volume_lots: convert(snapshot.volume, "volume_lots"),
        volume_shares: None,
        trade_value: None,
        transaction: None,
        source_site: Some(snapshot.source_site.clone()),
    }
}

/// 以 `"DailyQuotes"` 最新一個交易日計算排行（非交易時段路徑）。
///
/// 先取最新交易日再查該日資料，而不是一條 SQL 內嵌子查詢：一來 `data_as_of`
/// 需要這個日期（即使排行為空也要回），二來把日期綁成參數可讓 planner 直接
/// 用 `"DailyQuotes_Date_include_symbol_idx"` 定位單日資料。
async fn closing_movers(
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

/// 判斷單一股票的市場 id 是否符合查詢條件。
///
/// `0` 是 handler 內部代表「上市＋上櫃」的哨兵（§3.6）：`stocks` 表沒有市場
/// 0 的列，且排行刻意排除興櫃（5）與公開發行（1）——興櫃缺乏可靠成交資料，
/// 混進排行會產生誤導性名次。
fn market_matches(requested_market_id: i32, stock_market_id: i32) -> bool {
    if requested_market_id == 0 {
        matches!(stock_market_id, 2 | 4)
    } else {
        stock_market_id == requested_market_id
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
const CLOSING_MOVERS_SQL: &str = r#"
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
    OR s.stock_exchange_market_id = $2)
  AND s."SuspendListing" = false
  AND q."TradingVolume" > 0
"#;

/// 收盤來源排行的資料庫列。
#[derive(sqlx::FromRow)]
struct ClosingMoverRow {
    /// 股票代號。
    stock_symbol: String,
    /// 股票名稱。
    name: String,
    /// 市場 id（上市 2、上櫃 4）。
    market_id: i32,
    /// 產業分類 id。
    industry_id: i32,
    /// 收盤價。
    price: Decimal,
    /// 漲跌（元）。
    change: Decimal,
    /// 漲跌幅（%）。
    change_percent: Decimal,
    /// 開盤價。
    open_price: Decimal,
    /// 最高價。
    high_price: Decimal,
    /// 最低價。
    low_price: Decimal,
    /// 成交股數。
    volume_shares: Decimal,
    /// 成交金額（元）。
    trade_value: Decimal,
    /// 成交筆數。
    transaction_count: Decimal,
}

impl ClosingMoverRow {
    /// 加入查詢結果順序產生的一起始名次並轉成排行 DTO。
    fn into_dto(self, rank: u32) -> MarketMover {
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

#[cfg(test)]
mod tests;
