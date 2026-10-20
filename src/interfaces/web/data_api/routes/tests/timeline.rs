//! 個股事件時間軸 endpoint 的資料庫語意整合測試。

use axum::http::StatusCode;

use super::support::{get, pool_or_skip};

const SYMBOL: &str = "79979TL";

/// 刪除種入的各類事件與股票主檔。
async fn cleanup(pool: &sqlx::PgPool) {
    for sql in [
        "DELETE FROM dividend WHERE security_code = $1",
        "DELETE FROM corporate_action WHERE stock_symbol = $1",
        r#"DELETE FROM "Revenue" WHERE "SecurityCode" = $1"#,
        "DELETE FROM financial_statement WHERE security_code = $1",
        "DELETE FROM income_statement WHERE stock_symbol = $1",
        "DELETE FROM daily_technical_signal WHERE stock_symbol = $1",
        "DELETE FROM chip_daily WHERE stock_symbol = $1",
        "DELETE FROM insider_holding WHERE stock_symbol = $1",
        "DELETE FROM stocks WHERE stock_symbol = $1",
    ] {
        let _ = sqlx::query(sql).bind(SYMBOL).execute(pool).await;
    }
}

/// 1994 年的各類事件；查詢區間為 07-03～09-30、已公告事件列到 10-30。
async fn seed(pool: &sqlx::PgPool) {
    for sql in [
        r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_exchange_market_id, stock_industry_id)
           VALUES ('79979TL', '時間軸測試', '79979TL', 2, 24)"#,
        // 1993 年度股利：07-20 除息 2.5 元、09-01 除權 1 元、10-15 發放現金（已公告）、11-15 發放股票（超出）。
        r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, sum,
               "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
           VALUES ('79979TL', 1994, 1993, '', 2.5, 1, 3.5, '1994-07-20', '1994-09-01', '1994-10-15', '1994-11-15')"#,
        // 沒有日期（-）與金額為 0 的不列。
        r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, sum,
               "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2)
           VALUES ('79979TL', 1994, 1994, 'Q1', 0, 0, 0, '-', '1994-08-01', '-', '-')"#,
        r#"INSERT INTO corporate_action (stock_symbol, effective_date, action_type, share_ratio, note)
           VALUES ('79979TL', '1994-10-20', 'capital_reduction', 0.8, '減資彌補虧損')"#,
        // 6 月營收 07-10 公布、7 月 08-10、8 月 09-10；9 月營收 10-10 公布，晚於迄日，不列。
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly", "ComparedWithLastMonth", "ComparedWithLastYearSameMonth")
           VALUES ('79979TL', '79979TL', 199406, 100000, 1, 10), ('79979TL', '79979TL', 199407, 200000, 100, 20),
                  ('79979TL', '79979TL', 199408, 150000, -25, -5), ('79979TL', '79979TL', 199409, 150000, 0, 0)"#,
        // 1993-06～1994-05 每月 120,000：1994-07 的 200,000 高於之前 13 個月，創歷史新高（08-10）；
        // 1994-06（100,000）、1994-08（150,000）不是。這些月份的營收事件都早於起日。
        r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "Monthly")
           SELECT '79979TL', '79979TL', to_char(month, 'YYYYMM')::bigint, 120000
           FROM generate_series('1993-06-01'::date, '1994-05-01'::date, INTERVAL '1 month') AS month"#,
        // Yahoo 單季損益表：1994 Q2 三率（40／20／15%）都高於 1993 Q2（30／10／5%），三率三升（08-14）；
        // 1994 Q1 有升有降，不列。
        r#"INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, operating_profit, net_income)
           VALUES ('79979TL', 1994, 'single', 'Q2', 'yahoo', 100, 40, 20, 15),
                  ('79979TL', 1993, 'single', 'Q2', 'yahoo', 100, 30, 10, 5),
                  ('79979TL', 1994, 'single', 'Q1', 'yahoo', 100, 40, 5, 15),
                  ('79979TL', 1993, 'single', 'Q1', 'yahoo', 100, 30, 10, 5)"#,
        // Q2 財報期限 08-14；Q1（05-15）早於起日，不列。
        r#"INSERT INTO financial_statement (security_code, year, quarter, earnings_per_share, return_on_equity)
           VALUES ('79979TL', 1994, 'Q1', 1, 2), ('79979TL', 1994, 'Q2', 1.25, 3.5)"#,
        r#"INSERT INTO daily_technical_signal (date, stock_symbol, indicator, kind, description)
           VALUES ('1994-07-20', '79979TL', 'kd', 'golden_cross', 'KD 黃金交叉')"#,
        // 外資連買第 5 天（09-05）與第 6 天（不列）、投信連賣第 10 天（09-06）。
        r#"INSERT INTO chip_daily ("date", stock_symbol, foreign_net, trust_net, dealer_net, foreign_streak, trust_streak)
           VALUES ('1994-09-05', '79979TL', 3000, 0, 0, 5, 0), ('1994-09-06', '79979TL', 1000, -2000, 0, 6, -10)"#,
        // 董監持股：7 月 1,000 股、質押 100；8 月 1,000 股、質押 200（質押比例 +10 個百分點，09-15 申報）。
        r#"INSERT INTO insider_holding (month, stock_symbol, title, name, shares, pledged)
           VALUES ('1994-07-01', '79979TL', '董事長', '甲', 1000, 100), ('1994-08-01', '79979TL', '董事長', '甲', 1000, 200)"#,
    ] {
        sqlx::query(sql)
            .execute(pool)
            .await
            .expect("種入時間軸資料");
    }
}

/// 期間內的各類事件依日期合併、由新到舊；已公告的事件標記 upcoming；種類篩選、筆數上限與參數驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn stock_timeline_merges_events() {
    let Some(pool) = pool_or_skip("stock_timeline_merges_events").await else {
        return;
    };
    cleanup(pool).await;
    seed(pool).await;

    let base = format!("/api/v1/stocks/{SYMBOL}/timeline?date=1994-09-30&days=90&upcoming_days=30");
    let (status, json) = get(&base).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["name"], "時間軸測試");
    assert_eq!(json["from"], "1994-07-03");
    assert_eq!(json["upcoming_to"], "1994-10-30");
    let events: Vec<(String, String, String, bool)> = json["events"]
        .as_array()
        .expect("events array")
        .iter()
        .map(|event| {
            (
                event["date"].as_str().unwrap_or_default().to_owned(),
                event["kind"].as_str().unwrap_or_default().to_owned(),
                event["title"].as_str().unwrap_or_default().to_owned(),
                event["upcoming"].as_bool().unwrap_or_default(),
            )
        })
        .collect();
    let expected = [
        (
            "1994-10-20",
            "corporate_action",
            "減資生效（換發比例 0.8）",
            true,
        ),
        ("1994-10-15", "cash_payable", "現金股利發放 2.5 元", true),
        ("1994-09-15", "insider", "1994-08 董監持股 0%", false),
        (
            "1994-09-10",
            "monthly_revenue",
            "1994-08 營收年增 -5%",
            false,
        ),
        ("1994-09-06", "chip_streak", "投信連賣 10 天", false),
        ("1994-09-05", "chip_streak", "外資連買 5 天", false),
        ("1994-09-01", "ex_right", "除權：股票股利 1 元", false),
        (
            "1994-08-14",
            "quarterly_report",
            "1994Q2 財報：EPS 1.25 元",
            false,
        ),
        ("1994-08-14", "margin_trend", "1994Q2 三率三升", false),
        (
            "1994-08-10",
            "monthly_revenue",
            "1994-07 營收年增 +20%",
            false,
        ),
        (
            "1994-08-10",
            "revenue_record",
            "1994-07 營收創歷史新高",
            false,
        ),
        ("1994-07-20", "ex_dividend", "除息：現金股利 2.5 元", false),
        ("1994-07-20", "signal", "KD 黃金交叉", false),
        (
            "1994-07-10",
            "monthly_revenue",
            "1994-06 營收年增 +10%",
            false,
        ),
    ];
    assert_eq!(
        events,
        expected.map(|(date, kind, title, upcoming)| (
            date.to_owned(),
            kind.to_owned(),
            title.to_owned(),
            upcoming
        )),
        "{json}"
    );
    assert_eq!(json["total"], 14);
    assert_eq!(json["events"][2]["detail"], "質押比例 20%（+10 個百分點）");
    assert_eq!(json["events"][3]["detail"], "營收 1.5 億元、月增 -25%");
    assert_eq!(
        json["events"][8]["detail"],
        "毛利率 40%、營益率 20%、淨利率 15%"
    );
    assert_eq!(json["events"][10]["detail"], "營收 2 億元，前高 1.2 億元");
    assert_eq!(json["events"][11]["detail"], "1993 年度股利");
    assert_eq!(
        json["counts"][0],
        serde_json::json!({"kind": "ex_dividend", "count": 1})
    );
    assert_eq!(
        json["counts"][3],
        serde_json::json!({"kind": "corporate_action", "count": 1})
    );

    // 種類篩選與筆數上限：counts／total 不受 limit 影響。
    let (status, json) = get(&format!("{base}&kinds=monthly_revenue,signal&limit=2")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 4);
    assert_eq!(json["events"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        json["counts"],
        serde_json::json!([{"kind": "monthly_revenue", "count": 3}, {"kind": "signal", "count": 1}])
    );
    // 不列已公告事件。
    let (_, json) = get(&format!(
        "/api/v1/stocks/{SYMBOL}/timeline?date=1994-09-30&days=90&upcoming_days=0"
    ))
    .await;
    assert_eq!(json["total"], 12);

    // 金融保險業不列三率事件，營收新高照列。
    sqlx::query("UPDATE stocks SET stock_industry_id = 17 WHERE stock_symbol = $1")
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("改為金融保險業");
    let (_, json) = get(&format!("{base}&kinds=margin_trend,revenue_record")).await;
    assert_eq!(
        json["counts"],
        serde_json::json!([{"kind": "revenue_record", "count": 1}])
    );

    for (query, expected) in [
        ("days=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("days=731", StatusCode::UNPROCESSABLE_ENTITY),
        ("upcoming_days=181", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=0", StatusCode::UNPROCESSABLE_ENTITY),
        ("limit=501", StatusCode::UNPROCESSABLE_ENTITY),
        ("kinds=earnings_call", StatusCode::UNPROCESSABLE_ENTITY),
        ("date=1994-13-01", StatusCode::UNPROCESSABLE_ENTITY),
    ] {
        let (status, json) = get(&format!("/api/v1/stocks/{SYMBOL}/timeline?{query}")).await;
        assert_eq!(status, expected, "{query}: {json}");
    }
    let (status, _) = get("/api/v1/stocks/79979ZZ/timeline").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    cleanup(pool).await;
}
