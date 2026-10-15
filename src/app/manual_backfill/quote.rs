//! 手動回補：日報價、收盤匯總、均線、大盤指數、官方本益比與除權息漲跌。
//!
//! - `test_backfill_daily_quotes_for_date`：
//!   依 [`MANUAL_DAILY_QUOTE_DATE`] 重新抓取上市櫃各股每日收盤報價，寫入 `DailyQuotes`。
//! - `test_recalculate_moving_averages`：
//!   重算日報價的均線與年內統計（5～240 日均線、年內最高／最低／平均價）。
//! - `test_backfill_closing_aggregate_for_date`：
//!   依 [`MANUAL_CLOSING_AGGREGATE_DATE`] 重跑每日收盤事件匯總，包含收盤報價回補、
//!   缺漏補齊、均線、最後交易日報價、估價、殖利率排行與市值重算。
//! - `test_backfill_taiwan_stock_index`：
//!   依 [`MANUAL_TAIWAN_STOCK_INDEX_DATE`] 回補指定日期的台股加權指數，
//!   跳過快取檢查後 upsert 寫入 `Index` 並更新快取。
//! - `test_backfill_quote_history_for_symbols`：
//!   依 [`MANUAL_QUOTE_HISTORY_SYMBOLS`] 與月份區間（皆可用同名環境變數覆寫），
//!   從個股月行情（上市 TWSE、上櫃櫃買）回補歷史日報價缺口（只補空位，不覆寫既有資料）。
//! - `test_backfill_official_valuation_ratios`：
//!   依 `MANUAL_OFFICIAL_RATIO_FROM`～`MANUAL_OFFICIAL_RATIO_TO`（預設 [`MANUAL_OFFICIAL_RATIO_FROM`]
//!   ～[`MANUAL_OFFICIAL_RATIO_TO`]）以證交所、櫃買中心官方值補日線為 0 的本益比與股價淨值比
//!   （只填 0、不覆寫）；每次請求間隔 3 秒，可分年分段執行，重跑只會處理仍有缺值的日期。
//! - `test_backfill_ex_right_changes`：
//!   以交易所除權除息計算結果表的開盤競價基準，重算 `MANUAL_EX_RIGHT_CHANGE_FROM`～
//!   `MANUAL_EX_RIGHT_CHANGE_TO`（預設 [`MANUAL_EX_RIGHT_CHANGE_FROM`] 至今天）除權息日為 0 的漲跌與
//!   漲跌幅，並同步調整市場廣度統計表的漲跌家數；按年請求，重跑安全。

use chrono::NaiveDate;

use crate::app::backfill::quote;
use crate::app::backfill::quote_history;
use crate::app::backfill::taiwan_stock_index;
use crate::app::calculation::daily_quotes;
use crate::app::event::taiwan_stock::closing;
use crate::infra::cache::SHARE;

/// 手動回補各股每日收盤報價時使用的預設交易日。
const MANUAL_DAILY_QUOTE_DATE: &str = "2026-04-30";

/// 官方本益比／股價淨值比回補的預設起日（證交所 BWIBBU_d 與櫃買 peQryDate 皆可查到）。
const MANUAL_OFFICIAL_RATIO_FROM: &str = "2012-01-02";

/// 官方本益比／股價淨值比回補的預設迄日。
const MANUAL_OFFICIAL_RATIO_TO: &str = "2012-12-31";

/// 除權息日漲跌修正的預設起日（日線資料的第一年）。
const MANUAL_EX_RIGHT_CHANGE_FROM: &str = "2012-01-01";

/// 手動回補收盤事件匯總時使用的預設交易日。
const MANUAL_CLOSING_AGGREGATE_DATE: &str = "2026-04-30";

/// 手動回補歷史日報價的代號清單。
///
/// 空陣列表示自動採用「目前未下市、代號以 `00` 開頭」的全部 ETF／ETN ——
/// 2015–2021 的缺口正是整類代號，逐一列舉會漏。
const MANUAL_QUOTE_HISTORY_SYMBOLS: &[&str] = &[];

/// 手動回補歷史日報價的月份區間（含頭尾，日期部分會被忽略）。
const MANUAL_QUOTE_HISTORY_FROM: &str = "2015-01-01";

const MANUAL_QUOTE_HISTORY_TO: &str = "2021-12-01";

/// 手動回補指定交易日的各股每日收盤報價。
///
/// 此測試等同把原本的 `backfill::quote::tests::test_execute` 集中到手動回補檔。
/// 它會重新呼叫 TWSE 與 TPEx 來源抓取上市櫃各股開高低收、成交量與本益比等欄位，
/// 抓取成功後才在單一 transaction 內刪除同日舊資料並批次寫回資料庫、更新快取。
///
/// 執行範例：
/// `cargo test app::manual_backfill::quote::test_backfill_daily_quotes_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_daily_quotes_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_DAILY_QUOTE_DATE, "%Y-%m-%d")
        .expect("manual daily quote date should be valid");

    tracing::debug!(
        "開始 app::manual_backfill::quote::test_backfill_daily_quotes_for_date date={date}"
    );

    // quote::execute 內部採「先抓取、後原子替換」：同日舊資料的刪除與新資料的
    // COPY 寫入綁在同一個 transaction，抓取或寫入失敗都不會留下資料缺口，
    // 因此這裡不需要先手動刪除當日資料。
    let quote_count = quote::execute(date)
        .await
        .expect("manual daily quote backfill failed");

    // 替換過去某天的行情會改變之後最多 240 個交易日的均線，從當天起一併重算。
    let recalculated = daily_quotes::recalculate_moving_averages_since(date, None)
        .await
        .expect("recalculate moving averages failed");

    tracing::debug!(
        "結束 app::manual_backfill::quote::test_backfill_daily_quotes_for_date date={date} quote_count={quote_count} recalculated={recalculated}"
    );
}

/// 重算日報價的均線與年內統計（5～240 日均線、年內最高／最低／平均價）。
///
/// 每日收盤只算當天；回補或修正過去的行情後，補進的日子之後的均線不會自動更新，
/// 用這個入口從 `MANUAL_MA_FROM` 起重算。只更新數值有變的列，可重複執行。
/// `MANUAL_MA_SYMBOLS`（逗號分隔）留空代表這段期間有報價的全部代號。
///
/// 執行範例：
/// `MANUAL_MA_FROM=2019-07-01 cargo test app::manual_backfill::quote::test_recalculate_moving_averages -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_recalculate_moving_averages() {
    dotenvy::dotenv().ok();

    let value = std::env::var("MANUAL_MA_FROM").unwrap_or_else(|_| MANUAL_MA_FROM.to_string());
    let from = NaiveDate::parse_from_str(&value, "%Y-%m-%d")
        .unwrap_or_else(|_| panic!("MANUAL_MA_FROM should be YYYY-MM-DD, got {value}"));
    let symbols: Option<Vec<String>> = std::env::var("MANUAL_MA_SYMBOLS")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|symbol| !symbol.is_empty())
                .map(str::to_owned)
                .collect()
        });

    println!("開始 test_recalculate_moving_averages from={from} symbols={symbols:?}");

    let updated = daily_quotes::recalculate_moving_averages_since(from, symbols)
        .await
        .expect("recalculate moving averages failed");

    println!("結束 test_recalculate_moving_averages from={from} rows_updated={updated}");
}

/// 均線重算的預設起始日（可用環境變數 `MANUAL_MA_FROM` 覆寫）。
const MANUAL_MA_FROM: &str = "2012-01-01";

/// 手動執行每日收盤事件主要匯總流程。
///
/// 此測試等同把原本的 `event::taiwan_stock::closing::tests::test_aggregate`
/// 集中到手動回補檔。它會依指定交易日重跑收盤報價回補、缺漏補齊、均線、
/// last daily quote、估價、殖利率排行、市值重算與通知前置資料。
///
/// 執行範例：
/// `cargo test app::manual_backfill::quote::test_backfill_closing_aggregate_for_date -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_closing_aggregate_for_date() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_CLOSING_AGGREGATE_DATE, "%Y-%m-%d")
        .expect("manual closing aggregate date should be valid");

    tracing::debug!(
        "開始 app::manual_backfill::quote::test_backfill_closing_aggregate_for_date date={date}"
    );

    closing::aggregate(date)
        .await
        .expect("manual closing aggregate backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::quote::test_backfill_closing_aggregate_for_date date={date}"
    );
}

/// 手動回補台股加權指數時使用的預設日期。
///
/// TWSE API 會依此日期回傳該月份所有交易日的指數資料。
const MANUAL_TAIWAN_STOCK_INDEX_DATE: &str = "2026-04-15";

/// 手動回補指定月份的台股加權指數。
///
/// 此測試等同把原本的 `backfill::taiwan_stock_index::tests::test_execute`
/// 集中到手動回補檔。它會使用 [`MANUAL_TAIWAN_STOCK_INDEX_DATE`] 呼叫 TWSE
/// 加權股價指數來源，將該月份所有交易日的指數 upsert 回 `Index`，並更新記憶體快取。
/// 回補模式會跳過快取檢查，確保所有資料都寫入資料庫。
///
/// 執行範例：
/// `cargo test app::manual_backfill::quote::test_backfill_taiwan_stock_index -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_taiwan_stock_index() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let date = NaiveDate::parse_from_str(MANUAL_TAIWAN_STOCK_INDEX_DATE, "%Y-%m-%d")
        .expect("manual taiwan stock index date should be valid");

    tracing::debug!(
        "開始 app::manual_backfill::quote::test_backfill_taiwan_stock_index date={date}"
    );

    let upserted_count = taiwan_stock_index::execute_for_date(date)
        .await
        .expect("manual taiwan stock index backfill failed");

    tracing::debug!(
        "結束 app::manual_backfill::quote::test_backfill_taiwan_stock_index date={date} upserted_count={upserted_count}"
    );
}

/// 從個股月行情回補歷史日報價缺口（上市走 TWSE `STOCK_DAY`、上櫃走櫃買 `tradingStock`）。
///
/// 排程與 `test_backfill_daily_quotes_for_date` 都是「一天的全市場」，補七年份
/// 的缺口得跑一千七百多次、還會把不缺的股票一起重抓。這個入口改用個股月行情，
/// 一次要一檔股票的一整個月。
///
/// 寫入是 `ON CONFLICT DO NOTHING`：只填空位，既有資料不覆寫也不刪除，
/// 因此中途失敗直接重跑即可。單月抓取失敗只記錄並繼續。
///
/// 注意請求量：預設區間 84 個月 × 全部 ETF（約 360 檔）超過三萬次請求，
/// 每次間隔 1.2 秒，實際會跑十幾個小時。先用環境變數縮小到真正缺資料的代號與月份：
/// `MANUAL_QUOTE_HISTORY_SYMBOLS`（逗號分隔）、`MANUAL_QUOTE_HISTORY_FROM`／`MANUAL_QUOTE_HISTORY_TO`
/// （`YYYY-MM-DD`），未設定時沿用同名常數。
///
/// 執行範例：
/// `MANUAL_QUOTE_HISTORY_SYMBOLS=00679B MANUAL_QUOTE_HISTORY_FROM=2017-01-01 cargo test app::manual_backfill::quote::test_backfill_quote_history_for_symbols -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_quote_history_for_symbols() {
    dotenvy::dotenv().ok();
    SHARE.load().await;

    let parse = |name: &str, default: &str| {
        let value = std::env::var(name).unwrap_or_else(|_| default.to_string());
        NaiveDate::parse_from_str(&value, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} should be YYYY-MM-DD, got {value}"))
    };
    let from = parse("MANUAL_QUOTE_HISTORY_FROM", MANUAL_QUOTE_HISTORY_FROM);
    let to = parse("MANUAL_QUOTE_HISTORY_TO", MANUAL_QUOTE_HISTORY_TO);

    let symbols: Vec<String> = match std::env::var("MANUAL_QUOTE_HISTORY_SYMBOLS") {
        Ok(value) if !value.trim().is_empty() => value
            .split(',')
            .map(str::trim)
            .filter(|symbol| !symbol.is_empty())
            .map(str::to_owned)
            .collect(),
        _ if MANUAL_QUOTE_HISTORY_SYMBOLS.is_empty() => quote_history::fetch_etf_symbols()
            .await
            .expect("fetch etf symbols failed"),
        _ => MANUAL_QUOTE_HISTORY_SYMBOLS
            .iter()
            .map(|symbol| (*symbol).to_owned())
            .collect(),
    };

    println!(
        "開始 test_backfill_quote_history_for_symbols symbols={} from={from} to={to}",
        symbols.len()
    );

    let summary = quote_history::execute(&symbols, from, to)
        .await
        .expect("manual quote history backfill failed");

    println!(
        "結束 test_backfill_quote_history_for_symbols months_requested={} months_with_data={} months_failed={} quotes_fetched={} rows_inserted={} rows_recalculated={}",
        summary.months_requested,
        summary.months_with_data,
        summary.months_failed,
        summary.quotes_fetched,
        summary.rows_inserted,
        summary.rows_recalculated
    );
}

/// 以證交所、櫃買中心官方值回補日線為 0 的本益比與股價淨值比。
///
/// **會寫入 `DailyQuotes`**（只填 0、不覆寫既有值）。起訖日可用環境變數
/// `MANUAL_OFFICIAL_RATIO_FROM`／`MANUAL_OFFICIAL_RATIO_TO` 覆寫，一年約 250 個交易日 ×
/// 兩市場，每次請求間隔 3 秒，約 25 分鐘。
///
/// 執行範例：
/// `MANUAL_OFFICIAL_RATIO_FROM=2013-01-01 MANUAL_OFFICIAL_RATIO_TO=2013-12-31 cargo test app::manual_backfill::quote::test_backfill_official_valuation_ratios -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_official_valuation_ratios() {
    dotenvy::dotenv().ok();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_OFFICIAL_RATIO_FROM", MANUAL_OFFICIAL_RATIO_FROM);
    let to = date("MANUAL_OFFICIAL_RATIO_TO", MANUAL_OFFICIAL_RATIO_TO);

    let summary = crate::app::backfill::official_ratio::backfill_range(from, to)
        .await
        .expect("manual official valuation ratio backfill failed");
    println!("官方本益比淨值比回補 {from}～{to}：{summary:?}");
}

/// 以官方開盤競價基準重算除權息日的漲跌與漲跌幅，並同步調整市場廣度統計表的漲跌家數。
///
/// **會寫入日線與統計表**；只改漲跌為 0、有成交的除權息日，重跑安全。按年向證交所與
/// 櫃買中心各請求一次（間隔 3 秒）。起訖日可用環境變數 `MANUAL_EX_RIGHT_CHANGE_FROM`／
/// `MANUAL_EX_RIGHT_CHANGE_TO` 覆寫，迄日預設今天。
///
/// 執行範例：
/// `MANUAL_EX_RIGHT_CHANGE_FROM=2024-01-01 cargo test app::manual_backfill::quote::test_backfill_ex_right_changes -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_ex_right_changes() {
    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_EX_RIGHT_CHANGE_FROM", MANUAL_EX_RIGHT_CHANGE_FROM);
    let to = date("MANUAL_EX_RIGHT_CHANGE_TO", &today);

    let summary = crate::app::backfill::ex_right_change::backfill_range(from, to)
        .await
        .expect("manual ex-right change backfill failed");
    println!("除權息日漲跌修正 {from}～{to}：{summary:?}");
}
