//! ETF 專區查詢的 SQL 組裝與資料庫語意測試。

use chrono::NaiveDate;

use super::*;
use crate::domain::etf::query::EtfSort;

/// 測試用的假代號；寫入後一律清除。
const FAKE_A: &str = "79979EA";
const FAKE_B: &str = "79979EB";

fn filter() -> EtfListFilter {
    EtfListFilter {
        strategies: Vec::new(),
        region: None,
        frequency: None,
        market_id: None,
        keyword: None,
        sort: EtfSort::TradeValue,
        descending: true,
        limit: 20,
    }
}

fn day(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 4, d).expect("日期應合法")
}

#[test]
fn list_query_binds_values_and_whitelists_sort() {
    let all = list_query(&filter()).into_sql().as_str().to_owned();
    assert!(all.contains("ORDER BY s.trade_value DESC NULLS LAST, s.stock_symbol LIMIT $1"));
    assert!(!all.contains("strategy = ANY"));

    let narrowed = list_query(&EtfListFilter {
        strategies: vec!["bond".to_string()],
        region: Some("美國".to_string()),
        frequency: Some("monthly".to_string()),
        market_id: Some(4),
        keyword: Some("美債%".to_string()),
        sort: EtfSort::Yield12m,
        descending: false,
        limit: 5,
    })
    .into_sql()
    .as_str()
    .to_owned();
    for fragment in [
        "s.strategy = ANY($1)",
        "s.region = $2",
        "s.dividend_frequency = $3",
        "s.market_id = $4",
        "lower($5)) > 0",
        "ORDER BY s.yield_12m ASC NULLS LAST, s.stock_symbol LIMIT $6",
    ] {
        assert!(narrowed.contains(fragment), "缺少 {fragment}：{narrowed}");
    }
    assert!(!narrowed.contains("美債"), "關鍵字只能綁定參數");
}

async fn cleanup() {
    for sql in [
        "DELETE FROM etf_snapshot WHERE stock_symbol = ANY($1)",
        "DELETE FROM etf_profile WHERE stock_symbol = ANY($1)",
        "DELETE FROM etf_performance WHERE stock_symbol = ANY($1)",
        "DELETE FROM etf_holding WHERE stock_symbol = ANY($1)",
        "DELETE FROM etf_allocation WHERE stock_symbol = ANY($1)",
        "DELETE FROM etf_daily_nav WHERE stock_symbol = ANY($1)",
    ] {
        sqlx::query(sql)
            .bind(vec![FAKE_A, FAKE_B])
            .execute(database::get_connection())
            .await
            .expect("清除測試資料");
    }
}

#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
)]
async fn list_and_detail_read_prepared_tables() {
    dotenvy::dotenv().ok();
    cleanup().await;
    let pool = database::get_connection();
    sqlx::query(
        r#"INSERT INTO etf_snapshot (stock_symbol, name, market_id, strategy, region, themes, dividend_frequency,
                                     issuer, trade_value, yield_12m, premium_rate)
           VALUES ($1, '測試美債 50%', 4, 'bond', '美國', '{利率}', 'monthly', '測試投信', 100, 4.5, -0.3),
                  ($2, '測試高股息', 2, 'high_dividend', '台灣', '{}', 'quarterly', '測試投信', 300, 8.1, 0.2)"#,
    )
    .bind(FAKE_A)
    .bind(FAKE_B)
    .execute(pool)
    .await
    .expect("寫入快照");
    sqlx::query(
        "INSERT INTO etf_profile (stock_symbol, full_name, is_active, management_fee, source)
         VALUES ($1, '測試美債基金', false, '0.15%', 'yahoo')",
    )
    .bind(FAKE_A)
    .execute(pool)
    .await
    .expect("寫入基本資料");
    sqlx::query(
        "INSERT INTO etf_performance (stock_symbol, calc_date, ret_1y, nav_calc_date, nav_ret_1y)
         VALUES ($1, $2, -2.5, $2, -2.2)",
    )
    .bind(FAKE_A)
    .bind(day(30))
    .execute(pool)
    .await
    .expect("寫入績效");
    sqlx::query(
        "INSERT INTO etf_holding (stock_symbol, rank, as_of, holding_symbol, holding_name, weight)
         VALUES ($1, 1, $2, '', 'US TREASURY', 5.25)",
    )
    .bind(FAKE_A)
    .bind(day(1))
    .execute(pool)
    .await
    .expect("寫入持股");
    sqlx::query(
        "INSERT INTO etf_allocation (stock_symbol, kind, label, as_of, weight, sort_order)
         VALUES ($1, 'asset', '債券', $2, 99.1, 1), ($1, 'industry', '政府公債', $2, 98.0, 1)",
    )
    .bind(FAKE_A)
    .bind(day(1))
    .execute(pool)
    .await
    .expect("寫入配置");
    sqlx::query(
        "INSERT INTO etf_daily_nav (date, stock_symbol, nav, market_price, premium_rate)
         VALUES ($2, $1, 25.0, 24.9, -0.4), ($3, $1, NULL, 25.1, 0.1)",
    )
    .bind(FAKE_A)
    .bind(day(29))
    .bind(day(30))
    .execute(pool)
    .await
    .expect("寫入淨值");
    sqlx::query(
        "UPDATE etf_daily_nav SET estimated_nav = 25.05 WHERE stock_symbol = $1 AND date = $2",
    )
    .bind(FAKE_A)
    .bind(day(30))
    .execute(pool)
    .await
    .expect("寫入預估淨值");

    let repo = PgEtfQueryRepository::new();
    // 關鍵字含 %：不當萬用字元，只比對到名稱有「50%」的那一檔。
    let (total, items) = repo
        .list_etfs(&EtfListFilter {
            keyword: Some("50%".to_string()),
            ..filter()
        })
        .await
        .expect("關鍵字查詢");
    assert_eq!((total, items.len()), (1, 1));
    assert_eq!(items[0].stock_symbol, FAKE_A);

    let (_, mine) = repo
        .list_etfs(&EtfListFilter {
            keyword: Some("測試投信".to_string()),
            sort: EtfSort::Yield12m,
            ..filter()
        })
        .await
        .expect("排序");
    let order: Vec<&str> = mine.iter().map(|e| e.stock_symbol.as_str()).collect();
    assert_eq!(order, vec![FAKE_B, FAKE_A]);

    let (total, bonds) = repo
        .list_etfs(&EtfListFilter {
            strategies: vec!["bond".to_string()],
            market_id: Some(4),
            keyword: Some("測試".to_string()),
            limit: 1,
            ..filter()
        })
        .await
        .expect("篩選");
    assert_eq!(total, 1);
    assert_eq!(bonds[0].themes, vec!["利率".to_string()]);

    let detail = repo
        .etf_detail(FAKE_A, 20)
        .await
        .expect("明細")
        .expect("有這檔");
    assert_eq!(detail.overview.premium_rate, Some(-0.3));
    assert_eq!(
        detail.profile.as_ref().map(|p| p.full_name.as_str()),
        Some("測試美債基金")
    );
    let perf = detail.performance.expect("有績效");
    assert_eq!((perf.ret_1y, perf.nav_ret_1y), (Some(-2.5), Some(-2.2)));
    let kinds: Vec<&str> = detail.weights.iter().map(|w| w.kind.as_str()).collect();
    assert_eq!(kinds, vec!["holding", "industry", "asset"]);
    // 由新到舊；當天沒有正式淨值時用預估淨值。
    assert_eq!(
        detail
            .navs
            .iter()
            .map(|n| (n.date, n.nav))
            .collect::<Vec<_>>(),
        vec![(day(30), Some(25.05)), (day(29), Some(25.0))]
    );

    let bare = repo
        .etf_detail(FAKE_B, 20)
        .await
        .expect("明細")
        .expect("有這檔");
    assert!(bare.profile.is_none() && bare.performance.is_none() && bare.weights.is_empty());
    assert!(
        repo.etf_detail("79979XX", 20)
            .await
            .expect("查詢")
            .is_none()
    );
    cleanup().await;
}
