//! 選股 endpoint 財務體質與營收動能篩選條件的資料庫語意整合測試。

use rust_decimal::Decimal;

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

/// 條件選股的財務體質條件：新鮮的 F 分數、毛利率、負債比與自由現金流參與篩選與輸出；
/// 評分年度過舊時 F 分數轉成 null、不再符合 F 分數條件，但保留評分年度。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_screen_financial_health_filters() {
    use chrono::Datelike;

    let Some(pool) = pool_or_skip("stock_screen_financial_health_filters").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;
    // 新鮮度以查詢當天判斷，期間只能由今天推算；列屬於假代號，結束時清除。
    let today = chrono::Local::now().date_naive();
    let quarter = i16::try_from(today.month0() / 3 + 1).expect("季別 1–4");
    sqlx::query(
        "INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated, quarter_year, quarter, gross_margin_percent, debt_ratio_percent, free_cash_flow_ttm) VALUES ($1, $2, 8, 9, $3, $4, 42.5, 35.25, 1200)",
    )
    .bind(SYMBOL)
    .bind(today.year() - 1)
    .bind(today.year())
    .bind(quarter)
    .execute(pool)
    .await
    .expect("種入 financial_health");

    let listed = |json: &serde_json::Value| {
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .find(|stock| stock["stock_symbol"] == SYMBOL)
            .cloned()
    };
    let base = "/api/v1/stocks/screen?industry_id=1&limit=50";

    let (status, json) = get(&format!(
        "{base}&min_f_score=7&min_gross_margin_percent=40&max_debt_ratio_percent=50&positive_free_cash_flow=true&sort_by=f_score&sort_order=desc"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let stock = listed(&json).expect("符合條件的假代號應出現");
    assert_eq!(stock["f_score"], 8);
    assert_eq!(stock["f_score_evaluated"], 9);
    assert_eq!(stock["f_score_year"], today.year() - 1);
    assert_eq!(stock["gross_margin_percent"], 42.5);
    assert_eq!(stock["debt_ratio_percent"], 35.25);
    assert_eq!(stock["free_cash_flow_ttm"], 1200);
    assert_eq!(
        stock["health_period"],
        format!("{}-Q{quarter}", today.year())
    );

    for query in [
        "min_f_score=9",
        "min_gross_margin_percent=50",
        "max_debt_ratio_percent=30",
    ] {
        let (status, json) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert!(listed(&json).is_none(), "{query} 應排除假代號");
    }
    for query in [
        "min_f_score=10",
        "max_debt_ratio_percent=-1",
        "sort_by=f_scores",
    ] {
        let (status, _) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    // 評分年度落後 3 年：F 分數過期轉成 null、不符合 F 分數條件，評分年度照樣回傳。
    sqlx::query("UPDATE financial_health SET f_score_year = $2 WHERE stock_symbol = $1")
        .bind(SYMBOL)
        .bind(today.year() - 3)
        .execute(pool)
        .await
        .expect("改成過期的評分年度");
    let (_, json) = get(&format!("{base}&min_f_score=0")).await;
    assert!(listed(&json).is_none(), "過期的 F 分數不應符合條件");
    let (_, json) = get(&format!("{base}&positive_free_cash_flow=true")).await;
    let stock = listed(&json).expect("自由現金流仍新鮮，應出現");
    assert!(stock["f_score"].is_null(), "{stock}");
    assert_eq!(stock["f_score_year"], today.year() - 3);

    cleanup(pool).await;
}

/// 條件選股的營收動能：連續年增月數遇到年增 ≤ 0 或月份斷層就停止，
/// 近 3 月累計年增率以三個月合計營收對去年同期計算，三個月不相連時為 null。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_screen_revenue_momentum_filters() {
    use chrono::{Datelike, Months};

    let Some(pool) = pool_or_skip("stock_screen_revenue_momentum_filters").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool, &[SEED_STOCK]).await;
    // 新鮮度以查詢當天判斷，營收月份只能由今天往前推；列屬於假代號，結束時清除。
    let today = chrono::Local::now().date_naive();
    let month_code = |back: u32| {
        let month = today - Months::new(back);
        i64::from(month.year()) * 100 + i64::from(month.month())
    };
    // 由新到舊：年增 10、20、5、-3、8 → 連續 3 個月；近 3 月合計 335 對 300 → 11.67%。
    for (back, monthly, yoy) in [
        (1, 110, 10),
        (2, 120, 20),
        (3, 105, 5),
        (4, 97, -3),
        (5, 108, 8),
    ] {
        sqlx::query(
            r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "LastYearThisMonth", "ComparedWithLastYearSameMonth")
               VALUES ($1, $1, $2, $3, 100, $4)"#,
        )
        .bind(SYMBOL)
        .bind(month_code(back))
        .bind(Decimal::from(monthly))
        .bind(Decimal::from(yoy))
        .execute(pool)
        .await
        .expect("種入 Revenue");
    }

    let listed = |json: &serde_json::Value| {
        json["stocks"]
            .as_array()
            .expect("stocks 陣列")
            .iter()
            .find(|stock| stock["stock_symbol"] == SYMBOL)
            .cloned()
    };
    let base = "/api/v1/stocks/screen?industry_id=1&limit=50";

    let (status, json) = get(&format!(
        "{base}&min_revenue_yoy_streak=3&min_revenue_yoy_3m_percent=11&sort_by=revenue_yoy_streak&sort_order=desc"
    ))
    .await;
    assert_eq!(status, StatusCode::OK, "{json}");
    let stock = listed(&json).expect("符合條件的假代號應出現");
    assert_eq!(stock["revenue_yoy_streak"], 3);
    assert_eq!(stock["revenue_yoy_3m_percent"], 11.67);
    assert_eq!(stock["revenue_yoy_percent"], 10.0);
    assert_eq!(stock["monthly_revenue"], 110.0);
    let (_, json) = get(&format!(
        "{base}&min_monthly_revenue=110&sort_by=monthly_revenue"
    ))
    .await;
    assert!(listed(&json).is_some(), "月營收剛好等於下限應出現");

    for query in [
        "min_revenue_yoy_streak=4",
        "min_revenue_yoy_3m_percent=12",
        "min_monthly_revenue=111",
    ] {
        let (status, json) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::OK, "{query}");
        assert!(listed(&json).is_none(), "{query} 應排除假代號");
    }
    for query in [
        "min_revenue_yoy_streak=0",
        "min_revenue_yoy_streak=13",
        "min_revenue_yoy_3m_percent=-101",
        "min_monthly_revenue=-1",
        "sort_by=revenue_yoy_3months",
    ] {
        let (status, _) = get(&format!("{base}&{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    // 拿掉第二新的月份：連續月數停在 1，近 3 月不相連 → null。
    sqlx::query(r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1 AND "Date" = $2"#)
        .bind(SYMBOL)
        .bind(month_code(2))
        .execute(pool)
        .await
        .expect("刪除一個月份");
    let (_, json) = get(&format!("{base}&min_revenue_yoy_streak=1")).await;
    let stock = listed(&json).expect("仍有 1 個月年增，應出現");
    assert_eq!(stock["revenue_yoy_streak"], 1);
    assert!(stock["revenue_yoy_3m_percent"].is_null(), "{stock}");

    cleanup(pool).await;
}
