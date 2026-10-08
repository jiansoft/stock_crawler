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
            .map(|(symbol, _, _, change_range, volume)| snapshot(symbol, *change_range, *volume))
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
    let latest_date =
        latest_date.unwrap_or_else(|| NaiveDate::from_ymd_opt(2026, 4, 30).expect("固定日期合法"));
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
