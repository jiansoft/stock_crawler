//! 手動回補：收盤後衍生資料：未來報酬、每日相對強度、技術訊號。
//!
//! - `test_backfill_forward_returns`：
//!   逐日回填 `MANUAL_FORWARD_RETURN_FROM`～`MANUAL_FORWARD_RETURN_TO`（預設 [`MANUAL_FORWARD_RETURN_FROM`]
//!   至今天）每個交易日收盤時已滿 5／20／60／120／240 日的未來報酬；只讀資料庫，重跑安全。
//! - `test_backfill_relative_strength`：
//!   逐日寫入 `MANUAL_RELATIVE_STRENGTH_FROM`～`MANUAL_RELATIVE_STRENGTH_TO`（預設
//!   [`MANUAL_RELATIVE_STRENGTH_FROM`] 至今天）的每日相對強度（`daily_relative_strength`）；
//!   要先有未來報酬，整天替換，重跑安全。
//! - `test_backfill_technical_signals`：
//!   逐日重算全市場技術訊號（`daily_technical_signal`）。
//! - `test_backfill_market_breadth`：
//!   逐日補上 `MANUAL_MARKET_BREADTH_FROM`～`MANUAL_MARKET_BREADTH_TO`（預設 2019-01-02～2022-02-16）
//!   沒有統計列的市場廣度（`daily_stock_price_stats`）；已有的列不動，重跑安全。
//! - `test_backfill_market_valuation`：
//!   逐年重算 `MANUAL_MARKET_VALUATION_FROM`～`MANUAL_MARKET_VALUATION_TO`（預設 2012-01-02 至今天）每個
//!   交易日的市場估值水位（`daily_market_valuation`）；只讀日線，同一天同市場覆蓋，重跑安全。
//! - `test_record_strategy_rebalance`：
//!   把具名策略（`MANUAL_STRATEGY_PRESET`，預設「最佳組合」）目前這一期的持股寫進換股紀錄
//!   （`strategy_rebalance`），供實盤追蹤；同一天已有紀錄不覆蓋，重跑安全。

use chrono::NaiveDate;

/// 未來報酬回填的預設起日（結束日）：技術訊號從 2019 年起，以 2019 年初為結束日，5 日報酬的基準日
/// 從 2018 年底齊、240 日報酬的基準日從 2018 年初齊。
const MANUAL_FORWARD_RETURN_FROM: &str = "2019-01-02";

/// 每日相對強度回填的預設起日：240 日報酬的基準日從 2018 年初齊，相對強度從 2019 年初起算。
const MANUAL_RELATIVE_STRENGTH_FROM: &str = "2019-01-02";

/// 市場估值水位回補的預設起日：官方本益比與股價淨值比從 2012 年起回補。
const MANUAL_MARKET_VALUATION_FROM: &str = "2012-01-02";

/// 技術訊號回補的預設起日。
const MANUAL_TECHNICAL_SIGNAL_FROM: &str = "2026-09-01";

/// 技術訊號回補的預設迄日。
const MANUAL_TECHNICAL_SIGNAL_TO: &str = "2026-10-12";

/// 市場廣度回補的預設起日：與相對強度、技術訊號的歷史起點一致。
const MANUAL_MARKET_BREADTH_FROM: &str = "2019-01-02";

/// 市場廣度回補的預設迄日：`estimate` 從 2022-02-17 起才有資料，之後的統計由收盤流程產生。
const MANUAL_MARKET_BREADTH_TO: &str = "2022-02-16";

/// 逐日回填未來報酬（`daily_forward_return`）。
///
/// 以每個交易日為結束日，回填往前第 5、20、60、120、240 個交易日的還原報酬；只讀資料庫既有日線、
/// 股利與公司行動，不呼叫外部網站，非交易日自動略過。起訖日可用環境變數
/// `MANUAL_FORWARD_RETURN_FROM`／`MANUAL_FORWARD_RETURN_TO` 覆寫，迄日預設今天。
/// 基準日要從 X 開始齊全，起日需早於 X 約 240 個交易日（只需 60 日以內的天期時約 60 個）。
///
/// 執行範例：
/// `MANUAL_FORWARD_RETURN_FROM=2023-01-01 cargo test app::manual_backfill::derived::test_backfill_forward_returns -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_forward_returns() {
    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_FORWARD_RETURN_FROM", MANUAL_FORWARD_RETURN_FROM);
    let to = date("MANUAL_FORWARD_RETURN_TO", &today);

    let mut total = 0;
    for day in from.iter_days().take_while(|day| *day <= to) {
        let count = crate::app::calculation::forward_return::calculate_forward_returns(day)
            .await
            .unwrap_or_else(|why| panic!("{day} 未來報酬回填失敗：{why:#}"));
        total += count;
        if count > 0 && day.format("%d").to_string() == "01" {
            println!("{day} 累計 {total} 列");
        }
    }
    println!("未來報酬回填 {from}～{to}：共 {total} 列");
}

/// 重算每日大盤未來報酬彙總（`daily_market_forward_return`）。
///
/// 依基準日彙總 `daily_forward_return` 的筆數、報酬總和與上漲檔數（上市櫃全部與各市場），供訊號績效與
/// 法人連買績效的大盤基準讀取。每天的未來報酬寫入時會自動重算當天；建表後第一次、或直接改過
/// `daily_forward_return` 時才需要手動跑。只讀寫資料庫。起訖日可用 `MANUAL_MARKET_RETURN_FROM`／
/// `MANUAL_MARKET_RETURN_TO` 覆寫，預設 1990-01-01～今天。
///
/// 執行範例：
/// `cargo test app::manual_backfill::derived::test_backfill_market_forward_returns -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_market_forward_returns() {
    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_MARKET_RETURN_FROM", "1990-01-01");
    let to = date("MANUAL_MARKET_RETURN_TO", &today);
    let days =
        crate::infra::database::repository::forward_return::refresh_market_forward_returns_between(
            from, to,
        )
        .await
        .unwrap_or_else(|why| panic!("大盤彙總重算失敗：{why:#}"));
    println!("大盤未來報酬彙總 {from}～{to}：共 {days} 個基準日");
}

/// 逐日寫入每日相對強度（`daily_relative_strength`）。
///
/// 每個交易日以當天的近 60／120／240 日報酬（`daily_forward_return`）對上市櫃普通股（不含 ETF、特別股）計分，
/// 整天替換；報酬不齊或非交易日不寫入。只讀資料庫，不呼叫外部網站。起訖日可用環境變數
/// `MANUAL_RELATIVE_STRENGTH_FROM`／`MANUAL_RELATIVE_STRENGTH_TO` 覆寫，迄日預設今天。
///
/// 執行範例：
/// `MANUAL_RELATIVE_STRENGTH_FROM=2024-01-01 cargo test app::manual_backfill::derived::test_backfill_relative_strength -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_relative_strength() {
    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date(
        "MANUAL_RELATIVE_STRENGTH_FROM",
        MANUAL_RELATIVE_STRENGTH_FROM,
    );
    let to = date("MANUAL_RELATIVE_STRENGTH_TO", &today);

    let (mut days, mut total) = (0, 0);
    for day in from.iter_days().take_while(|day| *day <= to) {
        let count = crate::app::calculation::relative_strength::calculate_relative_strength(day)
            .await
            .unwrap_or_else(|why| panic!("{day} 相對強度寫入失敗：{why:#}"));
        if count > 0 {
            days += 1;
            total += count;
        }
        if day.format("%d").to_string() == "01" {
            println!("{day} 累計 {days} 個交易日、{total} 列");
        }
    }
    println!("相對強度回填 {from}～{to}：{days} 個交易日、共 {total} 列");
}

/// 逐年重算市場估值水位（`daily_market_valuation`）。
///
/// 只讀資料庫既有日線，同一天同市場覆蓋，重跑安全。起訖日可用環境變數
/// `MANUAL_MARKET_VALUATION_FROM`／`MANUAL_MARKET_VALUATION_TO` 覆寫。
///
/// 執行範例：
/// `cargo test app::manual_backfill::derived::test_backfill_market_valuation -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_market_valuation() {
    use crate::domain::market_index::valuation::MarketValuationRepository;
    use crate::infra::database::repository::market_valuation::PgMarketValuationRepository;

    dotenvy::dotenv().ok();
    let today = chrono::Local::now().date_naive().to_string();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_MARKET_VALUATION_FROM", MANUAL_MARKET_VALUATION_FROM);
    let to = date("MANUAL_MARKET_VALUATION_TO", &today);
    let repo = PgMarketValuationRepository::new();
    let mut total = 0;
    let mut start = from;
    while start <= to {
        let end = NaiveDate::from_ymd_opt(chrono::Datelike::year(&start), 12, 31)
            .expect("合法日期")
            .min(to);
        let rows = repo
            .refresh_market_valuation(start, end)
            .await
            .unwrap_or_else(|why| panic!("{start}～{end} 重算失敗：{why:#}"));
        println!("{start}～{end}：{rows} 列");
        total += rows;
        start = end.succ_opt().expect("合法日期");
    }
    println!("市場估值水位回補 {from}～{to}：共 {total} 列");
}

/// 逐日重算全市場技術訊號（`daily_technical_signal`）。
///
/// **會覆蓋指定期間每個交易日的訊號**；只讀資料庫既有日線，不呼叫外部網站。非交易日沒有
/// 有成交的股票，不會寫入任何資料。起訖日可用環境變數 `MANUAL_TECHNICAL_SIGNAL_FROM`／
/// `MANUAL_TECHNICAL_SIGNAL_TO` 覆寫。
///
/// 執行範例：
/// `MANUAL_TECHNICAL_SIGNAL_FROM=2026-09-01 cargo test app::manual_backfill::derived::test_backfill_technical_signals -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_technical_signals() {
    dotenvy::dotenv().ok();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_TECHNICAL_SIGNAL_FROM", MANUAL_TECHNICAL_SIGNAL_FROM);
    let to = date("MANUAL_TECHNICAL_SIGNAL_TO", MANUAL_TECHNICAL_SIGNAL_TO);

    for day in from.iter_days().take_while(|day| *day <= to) {
        let count = crate::app::calculation::technical_signal::calculate_technical_signals(day)
            .await
            .unwrap_or_else(|why| panic!("{day} 技術訊號計算失敗：{why:#}"));
        if count > 0 {
            println!("{day} 技術訊號 {count} 筆");
        }
    }
}

/// 逐日補上沒有統計列的市場廣度（`daily_stock_price_stats`）。
///
/// 只讀資料庫既有日線：均線上下家數、漲跌家數、52 週新高／新低；估值分布四欄沒有估價可比，寫 0。
/// 已有統計列的日子不動，非交易日沒有日線也不寫入，重跑安全。起訖日可用環境變數
/// `MANUAL_MARKET_BREADTH_FROM`／`MANUAL_MARKET_BREADTH_TO` 覆寫。
///
/// 執行範例：
/// `MANUAL_MARKET_BREADTH_FROM=2019-01-02 cargo test app::manual_backfill::derived::test_backfill_market_breadth -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_backfill_market_breadth() {
    dotenvy::dotenv().ok();
    let date = |name: &str, default: &str| {
        let raw = std::env::var(name).unwrap_or_else(|_| default.to_owned());
        NaiveDate::parse_from_str(&raw, "%Y-%m-%d")
            .unwrap_or_else(|_| panic!("{name} 格式錯誤：{raw}"))
    };
    let from = date("MANUAL_MARKET_BREADTH_FROM", MANUAL_MARKET_BREADTH_FROM);
    let to = date("MANUAL_MARKET_BREADTH_TO", MANUAL_MARKET_BREADTH_TO);

    let (mut days, mut total) = (0, 0);
    for day in from.iter_days().take_while(|day| *day <= to) {
        let count =
            crate::infra::database::table::quote::daily_stock_price_stats::DailyStockPriceStats::backfill_breadth(day)
                .await
                .unwrap_or_else(|why| panic!("{day} 市場廣度回補失敗：{why:#}"));
        if count > 0 {
            days += 1;
            total += count;
        }
        if day.format("%d").to_string() == "01" {
            println!("{day} 累計 {days} 個交易日、{total} 列");
        }
    }
    println!("市場廣度回補 {from}～{to}：{days} 個交易日、共 {total} 列");
}

/// 把具名策略目前這一期（最後一次換股、還沒滿持有天數）的持股寫進換股紀錄（`strategy_rebalance`）。
///
/// 換股通知啟用前的那一期沒有紀錄，用這個補上後 `/strategy-presets/{name}/track` 就能從這一期開始追蹤；
/// 之後的換股由 22:10 的通知照常記錄。**會寫入資料庫**；同策略同一天已有紀錄不覆蓋。
///
/// 執行範例：
/// `MANUAL_STRATEGY_PRESET=最佳組合 cargo test app::manual_backfill::derived::test_record_strategy_rebalance -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_record_strategy_rebalance() {
    dotenvy::dotenv().ok();
    crate::app::ports::register_strategy_gateway(std::sync::Arc::new(
        crate::interfaces::web::data_api::BacktestStrategyGateway,
    ));
    let preset = std::env::var("MANUAL_STRATEGY_PRESET").unwrap_or_else(|_| "最佳組合".to_owned());
    let result = crate::app::event::taiwan_stock::strategy_alert::record_current_rebalance(&preset)
        .await
        .expect("記錄目前換股失敗");
    match result {
        Some((date, true)) => println!("已記錄 {preset} {date} 的換股"),
        Some((date, false)) => println!("{preset} {date} 已有換股紀錄，保留原本的"),
        None => println!("{preset} 最後一期剛好滿持有天數，沒有目前持股可記錄"),
    }
}

/// 立即跑一次策略優勢滾動檢查（不看今天是不是當月第一個交易日）：對要通知的策略跑近 24 個月與
/// 2019 起的換股日偏移掃描，轉弱時發告警，結果寫進 Redis 供每週市場摘要讀取。不寫資料庫。
///
/// 執行範例：
/// `cargo test app::manual_backfill::derived::test_strategy_edge_check -- --ignored --nocapture`
#[tokio::test]
#[ignore]
async fn test_strategy_edge_check() {
    dotenvy::dotenv().ok();
    crate::app::ports::register_strategy_gateway(std::sync::Arc::new(
        crate::interfaces::web::data_api::BacktestStrategyGateway,
    ));
    crate::app::event::taiwan_stock::strategy_edge::run(chrono::Local::now().date_naive())
        .await
        .expect("策略優勢檢查失敗");
    for record in crate::app::event::taiwan_stock::strategy_edge::load_latest().await {
        println!("{record:?}");
    }
}
