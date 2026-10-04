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
mod tests {
    //! 排序白名單、市場過濾與即時排行的 deterministic tests；收盤來源的
    //! 執行計畫量測需要 PostgreSQL，以 `integration-tests` feature 控制。

    use std::collections::HashMap;

    use chrono::NaiveDate;
    use rust_decimal::Decimal;

    use super::{
        CLOSING_MOVERS_SQL, MoversRankBy, market_matches, rank_realtime_candidates,
        select_realtime_candidates,
    };
    use crate::domain::registry::entity::Stock;
    use crate::infra::{cache::RealtimeSnapshot, database};
    use crate::interfaces::web::data_api::dto::MarketMoversResponse;

    /// 排序鍵字串必須只接受三個固定值，且 SQL 片段永遠是程式內建字面值。
    ///
    /// 這個測試同時是一道安全防線：只要有人日後改成把呼叫端字串拼進
    /// `ORDER BY`，`closing_order_by` 的比對就會失敗。
    #[test]
    fn movers_rank_by_parses_only_whitelisted_values() {
        assert_eq!(
            MoversRankBy::parse("top_gainers"),
            Some(MoversRankBy::TopGainers)
        );
        assert_eq!(
            MoversRankBy::parse("top_losers"),
            Some(MoversRankBy::TopLosers)
        );
        assert_eq!(
            MoversRankBy::parse("top_volume"),
            Some(MoversRankBy::TopVolume)
        );
        // 刻意不支援的成交金額排行，以及任何注入嘗試都必須被擋下。
        for invalid in [
            "top_trade_value",
            "TOP_GAINERS",
            "",
            "top_gainers; DROP TABLE stocks",
        ] {
            assert!(MoversRankBy::parse(invalid).is_none(), "{invalid} 應被拒絕");
        }

        assert_eq!(MoversRankBy::TopGainers.as_str(), "top_gainers");
        assert_eq!(
            MoversRankBy::TopGainers.closing_order_by(),
            r#"ORDER BY q."ChangeRange" DESC, q.stock_symbol ASC"#
        );
        assert_eq!(
            MoversRankBy::TopLosers.closing_order_by(),
            r#"ORDER BY q."ChangeRange" ASC, q.stock_symbol ASC"#
        );
        assert_eq!(
            MoversRankBy::TopVolume.closing_order_by(),
            r#"ORDER BY q."TradingVolume" DESC, q.stock_symbol ASC"#
        );
        // 三個分支都必須以 stock_symbol 作為第二排序鍵，名次才會穩定。
        for rank_by in [
            MoversRankBy::TopGainers,
            MoversRankBy::TopLosers,
            MoversRankBy::TopVolume,
        ] {
            assert!(
                rank_by.closing_order_by().ends_with("stock_symbol ASC"),
                "{rank_by:?} 缺少穩定排序鍵"
            );
        }
    }

    /// `all`（哨兵 0）只含上市與上櫃，刻意排除興櫃與公開發行（§3.6）。
    #[test]
    fn market_matches_excludes_emerging_and_public_offering() {
        assert!(market_matches(0, 2));
        assert!(market_matches(0, 4));
        assert!(!market_matches(0, 5), "興櫃不得混入 all 排行");
        assert!(!market_matches(0, 1), "公開發行不得混入 all 排行");
        assert!(market_matches(2, 2));
        assert!(!market_matches(2, 4));
        assert!(market_matches(4, 4));
    }

    /// 建立測試用的股票主檔項目。
    fn stock(symbol: &str, market_id: i32, suspend_listing: bool) -> Stock {
        Stock::reconstitute(
            symbol.to_owned(),
            format!("主檔{symbol}"),
            suspend_listing,
            Decimal::ZERO,
            Decimal::ZERO,
            Decimal::ZERO,
            chrono::Local::now(),
            market_id,
            24,
            0,
            0,
            Decimal::ZERO,
        )
    }

    /// 建立測試用的即時快照。
    fn snapshot(symbol: &str, change_range: i64, volume: i64) -> RealtimeSnapshot {
        let mut snapshot = RealtimeSnapshot::new(symbol.to_owned(), Decimal::new(1000, 1));
        snapshot.name = format!("測試{symbol}");
        snapshot.source_site = "HiStock".to_owned();
        snapshot.change = Decimal::new(change_range, 0);
        snapshot.change_range = Decimal::new(change_range, 0);
        snapshot.volume = Decimal::new(volume, 0);
        snapshot
    }

    /// 即時排行的過濾、排序與 null 語意（movers 計畫 §4.4–§4.6）。
    ///
    /// 直接以本地的主檔與快照呼叫純函式，不碰全域 `SHARE`，因此不會被其他
    /// 測試清空即時快取而偶發失敗。
    #[test]
    fn realtime_movers_filters_sorts_and_keeps_unit_semantics() {
        // 測試資料設計：
        // 79971 上市、漲 5%、量 100      → 入選
        // 79972 上櫃、漲 9%、量 50       → 入選（漲幅最高）
        // 79973 上市但暫停上市、漲 20%   → 排除
        // 79974 興櫃、漲 15%             → 排除（all 不含興櫃）
        // 79975 上市、漲 8% 但成交量 0   → 排除（無成交）
        // 79976 上市、漲 5%、量 10       → 與 79971 同漲幅，驗證同值排序
        // 79977 主檔沒有、漲 30%         → 排除並回報為未知代號
        let fixtures = [
            ("79971", 2, false, 5, 100),
            ("79972", 4, false, 9, 50),
            ("79973", 2, true, 20, 30),
            ("79974", 5, false, 15, 30),
            ("79975", 2, false, 8, 0),
            ("79976", 2, false, 5, 10),
        ];
        let stocks: HashMap<String, Stock> = fixtures
            .iter()
            .map(|(symbol, market_id, suspend_listing, ..)| {
                (
                    (*symbol).to_owned(),
                    stock(symbol, *market_id, *suspend_listing),
                )
            })
            .collect();
        let snapshots = || {
            let mut snapshots: Vec<RealtimeSnapshot> = fixtures
                .iter()
                .map(|(symbol, _, _, change_range, volume)| {
                    snapshot(symbol, *change_range, *volume)
                })
                .collect();
            snapshots.push(snapshot("79977", 30, 10));
            snapshots
        };
        let rank = |rank_by: MoversRankBy, market: &str, market_id: i32, limit: u8| {
            let (candidates, unknown) = select_realtime_candidates(snapshots(), &stocks, market_id);
            assert_eq!(
                unknown,
                vec!["79977".to_owned()],
                "主檔查不到的代號必須排除"
            );
            rank_realtime_candidates(candidates, rank_by, market, limit)
        };
        let symbols = |response: &MarketMoversResponse| {
            response
                .movers
                .iter()
                .map(|mover| mover.stock_symbol.clone())
                .collect::<Vec<_>>()
        };

        // 漲幅榜：9% > 5%；同為 5% 時以代號由小到大穩定排序。
        let gainers = rank(MoversRankBy::TopGainers, "all", 0, 50);
        assert_eq!(symbols(&gainers), vec!["79972", "79971", "79976"]);
        assert_eq!(gainers.source, "realtime");
        assert!(gainers.is_realtime);
        assert_eq!(gainers.rank_by, "top_gainers");
        assert_eq!(gainers.market, "all");
        assert!(gainers.snapshot_updated_at.is_some());
        let ranks: Vec<u32> = gainers.movers.iter().map(|mover| mover.rank).collect();
        assert_eq!(ranks, vec![1, 2, 3]);

        // 跌幅榜：同一批資料由低到高，同值仍以代號排序。
        let losers = rank(MoversRankBy::TopLosers, "all", 0, 50);
        assert_eq!(symbols(&losers), vec!["79971", "79976", "79972"]);

        // 成交量榜：100 > 50 > 10。
        let volume = rank(MoversRankBy::TopVolume, "all", 0, 50);
        assert_eq!(symbols(&volume), vec!["79971", "79972", "79976"]);

        // market 條件：twse 只留上市，上櫃的 79972 必須消失。
        let twse = rank(MoversRankBy::TopGainers, "twse", 2, 50);
        assert_eq!(symbols(&twse), vec!["79971", "79976"]);
        assert_eq!(twse.market, "twse");

        // limit 截斷後名次仍從 1 起算。
        let limited = rank(MoversRankBy::TopGainers, "all", 0, 1);
        assert_eq!(symbols(&limited), vec!["79972"]);
        assert_eq!(limited.movers[0].rank, 1);

        // 單位與缺值語意：即時來源只有「張」，沒有股數／金額／筆數。
        let top = &gainers.movers[0];
        assert_eq!(top.volume_lots, Some(50.0));
        assert!(top.volume_shares.is_none(), "即時來源不得偽造成交股數");
        assert!(top.trade_value.is_none(), "即時來源不得偽造成交金額");
        assert!(top.transaction.is_none(), "即時來源不得偽造成交筆數");
        assert_eq!(top.source_site.as_deref(), Some("HiStock"));
        assert_eq!(top.market_id, 4);
        assert_eq!(top.industry_id, 24);
        // 快照有名稱時用快照名稱。
        assert_eq!(top.name, "測試79972");
    }

    /// 盤中路徑讀全域快取後，回應的來源標示必須固定為即時；只驗證中繼欄位，
    /// 不依賴快取內容（其他測試可能同時寫入或清空快取）。
    #[test]
    fn realtime_movers_reports_the_realtime_source() {
        let response = super::realtime_movers(MoversRankBy::TopVolume, "tpex", 4, 5);
        assert_eq!(response.source, "realtime");
        assert!(response.is_realtime);
        assert_eq!(response.rank_by, "top_volume");
        assert_eq!(response.market, "tpex");
        assert!(response.movers.len() <= 5);
        assert!(response.movers.iter().all(|mover| mover.market_id == 4));
        assert_eq!(response.data_as_of.len(), 10, "data_as_of 應為 YYYY-MM-DD");
    }

    /// 收盤來源的資料列轉成排行 DTO：張數、昨收與採集站點一律 null。
    #[test]
    fn closing_row_keeps_unit_semantics() {
        let row = super::ClosingMoverRow {
            stock_symbol: "79971".to_owned(),
            name: "測試79971".to_owned(),
            market_id: 2,
            industry_id: 24,
            price: Decimal::new(1055, 1),
            change: Decimal::new(55, 1),
            change_percent: Decimal::new(55, 1),
            open_price: Decimal::new(100, 0),
            high_price: Decimal::new(106, 0),
            low_price: Decimal::new(99, 0),
            volume_shares: Decimal::new(123_000, 0),
            trade_value: Decimal::new(12_976_500, 0),
            transaction_count: Decimal::new(321, 0),
        };
        let mover = row.into_dto(3);
        assert_eq!(mover.rank, 3);
        assert_eq!(mover.stock_symbol, "79971");
        assert_eq!(mover.price, Some(105.5));
        assert_eq!(mover.change_percent, Some(5.5));
        assert_eq!(mover.open, Some(100.0));
        assert_eq!(mover.volume_shares, Some(123_000.0));
        assert_eq!(mover.trade_value, Some(12_976_500.0));
        assert_eq!(mover.transaction, Some(321.0));
        assert!(mover.volume_lots.is_none(), "收盤來源沒有張數");
        assert!(mover.last_close.is_none(), "收盤來源沒有昨收");
        assert!(mover.source_site.is_none(), "收盤來源沒有採集站點");
    }

    /// 快照沒有名稱時改用股票主檔的名稱。
    #[test]
    fn realtime_movers_fall_back_to_the_registry_name() {
        let stocks = HashMap::from([("79971".to_owned(), stock("79971", 2, false))]);
        let mut unnamed = snapshot("79971", 1, 10);
        unnamed.name.clear();
        let (candidates, unknown) = select_realtime_candidates(vec![unnamed], &stocks, 0);
        assert!(unknown.is_empty());
        let response = rank_realtime_candidates(candidates, MoversRankBy::TopGainers, "all", 20);
        assert_eq!(response.movers[0].name, "主檔79971");
    }

    /// M0-2：對收盤來源排行查詢執行 EXPLAIN ANALYZE，記錄執行計畫與成本。
    ///
    /// 收盤排行的查詢型態是「取最新一個交易日的全部日線，JOIN 股票主檔後
    /// top-N 排序」。此測試輸出文字 plan 供人工記錄，並確認取最新交易日這
    /// 一步有用到 `"Date"` 的索引（全表掃描找 MAX 會隨資料量線性變慢）。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部 PostgreSQL，請加 --features integration-tests 執行"
    )]
    async fn movers_closing_query_plan_uses_date_index() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 M0-2 EXPLAIN：無資料庫連接");
            return;
        }
        let pool = database::get_connection();
        // 資料太少時 planner 一律選全表掃描（CI 的測試庫 `"DailyQuotes"` 是空的），
        // 不能據此判斷索引是否有效；這時仍執行 EXPLAIN 確認 SQL 可規劃，只略過索引斷言。
        let row_count: i64 = sqlx::query_scalar(r#"SELECT COUNT(*) FROM "DailyQuotes""#)
            .fetch_one(pool)
            .await
            .expect("DailyQuotes row count");

        // 第一步：取最新交易日。應走 "DailyQuotes_Date_include_symbol_idx"
        // 的反向掃描，而不是 Seq Scan。
        let latest_plan: Vec<String> = sqlx::query_scalar(
            r#"EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) SELECT MAX("Date") FROM "DailyQuotes""#,
        )
        .fetch_all(pool)
        .await
        .expect("latest date EXPLAIN");
        println!(
            "\n===== movers latest-date =====\n{}",
            latest_plan.join("\n")
        );
        if row_count >= 1_000 {
            assert!(
                latest_plan.join("\n").contains("Index"),
                "取最新交易日必須走索引，不可全表掃描"
            );
        } else {
            println!("DailyQuotes 只有 {row_count} 列，略過索引斷言");
        }

        // 第二步：實際排行查詢，三種排序鍵各跑一次。綁定的日期必須是資料庫
        // 真正的最新交易日——若隨手綁「今天」，遇到收盤資料尚未寫入的時段
        // 會掃到零列，量出來的執行計畫沒有參考價值。
        let latest_date: Option<NaiveDate> =
            sqlx::query_scalar(r#"SELECT MAX("Date") FROM "DailyQuotes""#)
                .fetch_one(pool)
                .await
                .expect("latest date");
        // 空表時改綁固定歷史日期，仍可驗證三種排序的 SQL 都能規劃與執行。
        let latest_date = latest_date
            .unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 4, 30).expect("固定日期合法"));
        println!("使用交易日 {latest_date} 量測排行查詢");
        for rank_by in [
            MoversRankBy::TopGainers,
            MoversRankBy::TopLosers,
            MoversRankBy::TopVolume,
        ] {
            let sql = format!(
                "EXPLAIN (ANALYZE, BUFFERS, FORMAT TEXT) {}\n{}\nLIMIT $3",
                CLOSING_MOVERS_SQL,
                rank_by.closing_order_by()
            );
            let plan: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(latest_date)
                .bind(0_i32)
                .bind(20_i64)
                .fetch_all(pool)
                .await
                .expect("movers EXPLAIN");
            assert!(!plan.is_empty(), "{rank_by:?} 應回傳 plan");
            println!(
                "\n===== movers {} =====\n{}",
                rank_by.as_str(),
                plan.join("\n")
            );
        }
    }
}
