//! 條件選股回測 endpoint 整合測試：籌碼集中（400 張以上大戶持股比例變化、融資餘額變化）只用換股日以前
//! 已公布的資料。

use super::*;

/// 種入 400 張大戶比例（%，千張比例固定 50）。03-08 與換股日同一天，換股日當天還看不到：
///
/// | 代號 | 02-19 | 02-26 | 03-05 | 03-08 |
/// |---|---|---|---|---|
/// | BA | 60 | 61 | 61 | 70 |
/// | BB | 60 | 59 | 58 | 80 |
/// | BC | 60 | 62 | 63 | 63 |
///
/// 融資餘額：02-06～02-26 每天一筆，加上 03-05、03-08（03-08 是換股日當天，收盤後才公布，不應用到）：
///
/// | 代號 | 02-06 | 02-07～02-26 | 03-05 | 03-08 |
/// |---|---|---|---|---|
/// | BA | 1000 | 800 | 900 | 100 |
/// | BB | 1000 | 1000 | 500 | 5000 |
/// | BC | 0 | 0 | 0 | 0 |
async fn seed_chip_concentration(pool: &sqlx::PgPool) {
    for (symbol, percents) in [
        ("79979BA", [60, 61, 61, 70]),
        ("79979BB", [60, 59, 58, 80]),
        ("79979BC", [60, 62, 63, 63]),
    ] {
        for (date, percent) in ["1993-02-19", "1993-02-26", "1993-03-05", "1993-03-08"]
            .into_iter()
            .zip(percents)
        {
            sqlx::query(
                r#"INSERT INTO holder_distribution ("date", stock_symbol, major_holders, major_percent, total_holders, percent_400)
                   VALUES ($1::date, $2, 10, 50, 100, $3)"#,
            )
            .bind(date)
            .bind(symbol)
            .bind(Decimal::from(percent))
            .execute(pool)
            .await
            .expect("種入集保週資料");
        }
    }
    let start = NaiveDate::from_ymd_opt(1993, 2, 6).expect("固定日期合法");
    for (symbol, [base, rest, mar5, mar8]) in [
        ("79979BA", [1000, 800, 900, 100]),
        ("79979BB", [1000, 1000, 500, 5000]),
        ("79979BC", [0, 0, 0, 0]),
    ] {
        let mut rows: Vec<(NaiveDate, i64)> = (0..21)
            .map(|offset| {
                let balance = if offset == 0 { base } else { rest };
                (start + chrono::Duration::days(offset), balance)
            })
            .collect();
        rows.push((day(5), mar5));
        rows.push((day(8), mar8));
        for (date, balance) in rows {
            sqlx::query(
                r#"INSERT INTO chip_daily ("date", stock_symbol, margin_balance) VALUES ($1, $2, $3)
                   ON CONFLICT ("date", stock_symbol) DO UPDATE SET margin_balance = EXCLUDED.margin_balance"#,
            )
            .bind(date)
            .bind(symbol)
            .bind(balance)
            .execute(pool)
            .await
            .expect("種入融資餘額");
        }
    }
}

/// 400 張大戶 1 週變化 ≥ 0.5：03-01 看 02-26 對 02-19（BA +1、BC +2）；03-08 看 03-05 對 02-26，只有 BC（+1）。
/// 融資變化 ≤ -10%：03-01 用 02-26 對 02-06（BA -20%）；03-08 用 03-05 對 02-07（BA +12.5%、BB -50%），
/// 不會用到換股日當天 03-08 的資料。回應回填參數，起日太早或超出範圍回 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn screen_backtest_chip_concentration_uses_published_data_only() {
    let Some(pool) = seeded("screen_backtest_chip_concentration_uses_published_data_only").await
    else {
        return;
    };
    seed_chip_concentration(pool).await;

    let (status, json) = query("limit=2&min_holder_400_change=0.5&major_change_weeks=1").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["min_holder_400_change"], 0.5);
    assert_eq!(json["major_change_weeks"], 1);
    assert!(json["min_major_change"].is_null(), "{json}");
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(6.0)),
            ("1993-03-08".to_owned(), 1, None),
        ]
    );
    // 第一週 02-19 加 2 週是 03-05，起日 03-01 不夠。
    let (early_status, early) =
        query("limit=2&min_holder_400_change=0.5&major_change_weeks=2").await;
    assert_eq!(early_status, StatusCode::UNPROCESSABLE_ENTITY, "{early}");
    assert!(
        early["error"]
            .as_str()
            .is_some_and(|message| message.contains("min_holder_400_change")),
        "{early}"
    );

    let (status, json) = query("limit=2&max_margin_change_percent=-10").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["max_margin_change_percent"], -10.0);
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 1, Some(10.0)),
            ("1993-03-08".to_owned(), 1, Some(8.0)),
        ]
    );
    // 上限 15%：03-01 BA（-20%）、BB（0%）；03-08 BA（+12.5%）、BB（-50%），BC 沒有融資。
    let (_, json) = query("limit=2&max_margin_change_percent=15").await;
    assert_eq!(
        periods(&json),
        [
            ("1993-03-01".to_owned(), 2, Some(3.0)),
            ("1993-03-08".to_owned(), 2, Some(5.5)),
        ]
    );

    let (_, plain) = query("limit=2").await;
    assert!(plain["min_holder_400_change"].is_null(), "{plain}");
    assert!(plain["max_margin_change_percent"].is_null(), "{plain}");
    for extra in [
        "min_holder_400_change=100.5",
        "max_margin_change_percent=-101",
    ] {
        let (status, json) = query(extra).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{extra}: {json}");
    }
    cleanup(pool).await;
}
