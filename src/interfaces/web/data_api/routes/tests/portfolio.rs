//! 持股摘要、持股歷史與股利預估 endpoints 的資料庫語意整合測試。

use rust_decimal::Decimal;

use super::support::{SEED_STOCK, SYMBOL, cleanup, get, pool_or_skip, seed};
use super::*;

/// 持股總覽：同代號多筆買進合併、已賣出不算；市值、損益、已領股利與市值加權指標，
/// 以及 member_id 篩選與 422。用不存在的會員編號隔離資料，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_summary_endpoint_db_semantics() {
    use chrono::Datelike;

    const MEMBER: i64 = 979_790;
    let Some(pool) = pool_or_skip("portfolio_summary_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_portfolio = || async {
        let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await;
        let _ = sqlx::query("DELETE FROM yield_rank WHERE security_code = $1")
            .bind(SYMBOL)
            .execute(pool)
            .await;
        cleanup(pool).await;
    };
    cleanup_portfolio().await;

    // 沒有持股：200、空清單。
    let path = format!("/api/v1/portfolio/summary?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["symbols"], 0);
    assert_eq!(json["holdings"], serde_json::json!([]));
    assert!(json["weighted_dividend_yield"].is_null(), "{json}");

    seed(pool, &[SEED_STOCK]).await;
    // 新鮮度以查詢當天判斷，殖利率日與評分年度只能由今天推算；列屬於假代號，結束時清除。
    let today = chrono::Local::now().date_naive();
    for sql in [
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, cumulate_dividends_total, is_sold) VALUES ($1, $2, 1000, -40000, 300, false), ($1, $2, 1000, -50000, 200, false), ($1, $2, 500, -99999, 999, true)",
        "INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ('2026-04-30', $2, 50)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .bind(SYMBOL)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入持股失敗：{error}\n{sql}"));
    }
    sqlx::query("INSERT INTO yield_rank (date, security_code, yield) VALUES ($1, $2, 4.5)")
        .bind(today)
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入 yield_rank");
    sqlx::query("INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated) VALUES ($1, $2, 6, 9)")
        .bind(SYMBOL)
        .bind(today.year() - 1)
        .execute(pool)
        .await
        .expect("種入 financial_health");

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["member_id"], MEMBER);
    assert_eq!(json["data_as_of"], "2026-04-30");
    // 兩筆未賣出合併：2,000 股 × 50＝100,000；成本 90,000；已賣出那筆不算。
    assert_eq!(
        (
            json["symbols"].clone(),
            json["market_value"].clone(),
            json["cost"].clone()
        ),
        (1.into(), 100_000.0.into(), 90_000.0.into())
    );
    assert_eq!(json["unrealized_gain"], 10_000.0);
    assert_eq!(json["unrealized_gain_percent"], 11.11);
    assert_eq!(json["dividends_received"], 500.0);
    assert_eq!(json["weighted_dividend_yield"]["value"], 4.5);
    assert_eq!(json["weighted_f_score"]["coverage_percent"], 100.0);
    let holding = &json["holdings"][0];
    assert_eq!(holding["stock_symbol"], SYMBOL);
    assert_eq!(
        (holding["shares"].clone(), holding["weight_percent"].clone()),
        (2_000.into(), 100.0.into())
    );
    assert_eq!(holding["f_score"], 6);
    // 假代號沒有相對強度與籌碼，欄位仍要出現（null）。
    for field in ["rs_score", "foreign_streak", "trust_streak"] {
        assert!(
            holding.get(field).is_some_and(serde_json::Value::is_null),
            "{field}：{holding}"
        );
    }
    for field in ["rs_as_of", "chip_as_of"] {
        assert!(json.get(field).is_some(), "{field}");
    }
    assert_eq!(json["industries"][0]["industry_id"], 1);

    let (status, _) = get("/api/v1/portfolio/summary?member_id=-1").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    cleanup_portfolio().await;
}

/// 持股績效：扣除買賣金額的時間加權報酬、事後補登的買進記在建立日、已撤銷的賣出不算、
/// 月報酬與最大回撤、同期大盤與超額報酬。用不存在的會員編號與 2099 年的日期隔離資料（全部會員的期間以最新日期往回推，
/// 用未來日期才能確保最新日期是測試資料，不受其他測試寫入的近期市值影響），
/// 大盤列以 `transaction = 979790` 標記，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_history_endpoint_db_semantics() {
    const MEMBER: i64 = 979_790;
    let Some(pool) = pool_or_skip("portfolio_history_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_history = || async {
        for sql in [
            "DELETE FROM daily_money_history_member WHERE member_id IN ($1, $1 + 1) OR (member_id = 0 AND date >= '2099-01-01')",
            "DELETE FROM stock_ownership_details WHERE member_id = $1",
            "DELETE FROM sell_history WHERE member_id = $1",
            "DELETE FROM index WHERE category = 'TAIEX' AND transaction = $1",
        ] {
            let _ = sqlx::query(sql).bind(MEMBER).execute(pool).await;
        }
    };
    cleanup_history().await;

    let path = format!("/api/v1/portfolio/history?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["return_percent"].is_null(), "{json}");
    assert_eq!(json["months"], serde_json::json!([]));

    for sql in [
        "INSERT INTO daily_money_history_member (date, member_id, market_value) VALUES ('2099-01-31', $1, 100), ('2099-02-01', $1, 110), ('2099-02-02', $1, 210), ('2099-03-01', $1, 99)",
        // 交易日 01-15（期間前）但 02-01 才補登：記在 02-01。
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, date, created_time) VALUES ($1, '79979DA', 1, -10, '2099-01-15', '2099-02-01 10:00+08'), ($1, '79979DA', 1, -100, '2099-02-02', '2099-02-02 10:00+08')",
        "INSERT INTO sell_history (member_id, security_code, date, amount, create_time, is_reverted) VALUES ($1, '79979DA', '2099-03-01', 100, '2099-03-01 10:00+08', false), ($1, '79979DA', '2099-03-01', 999, '2099-03-01 10:00+08', true)",
        "INSERT INTO index (category, date, index, transaction) VALUES ('TAIEX', '2099-01-31', 5000, $1), ('TAIEX', '2099-02-28', 5500, $1), ('TAIEX', '2099-03-01', 5250, $1)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入績效資料失敗：{error}\n{sql}"));
    }

    let (status, json) = get(&format!("{path}&months=12")).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(
        (json["from"].clone(), json["data_as_of"].clone()),
        ("2099-01-31".into(), "2099-03-01".into())
    );
    // 02-01：(110 − 10) ÷ 100；02-02：(210 − 100) ÷ 110；03-01：(99 + 100) ÷ 210。
    assert_eq!(json["return_percent"], -5.24);
    assert_eq!(
        (json["buys"].clone(), json["sells"].clone()),
        (110.0.into(), 100.0.into())
    );
    assert_eq!(json["max_drawdown_percent"], -5.24);
    assert_eq!(json["max_drawdown_peak"], "2099-01-31");
    assert_eq!(json["max_drawdown_trough"], "2099-03-01");
    assert_eq!(json["benchmark_return_percent"], 5.0);
    assert_eq!(json["excess_return_percent"], -10.24);
    assert!(json["annualized_return_percent"].is_null());
    // 只有 3 天報酬，不足 20 個成對交易日，沒有風險指標。
    for field in [
        "beta",
        "alpha_percent",
        "correlation",
        "volatility_percent",
        "risk_days",
    ] {
        assert!(json[field].is_null(), "{field}: {json}");
    }
    let months = json["months"].as_array().expect("months 陣列");
    assert_eq!(months.len(), 2);
    assert_eq!(
        (
            months[0]["month"].clone(),
            months[0]["return_percent"].clone(),
            months[0]["net_contribution"].clone()
        ),
        ("2099-02".into(), 0.0.into(), 110.0.into())
    );
    assert_eq!(months[1]["benchmark_return_percent"], 5.0);

    // 全部會員：另一位會員 02-02 才開始記錄（市值 50），合計當天多出的 50 視為投入。
    // 02-02：(260 − 100 − 50) ÷ 110；03-01：(149 + 100) ÷ 260。
    for sql in [
        "INSERT INTO daily_money_history_member (date, member_id, market_value) VALUES ('2099-02-02', $1 + 1, 50), ('2099-03-01', $1 + 1, 50)",
        "INSERT INTO daily_money_history_member (date, member_id, market_value) VALUES ('2099-01-31', 0, 100), ('2099-02-01', 0, 110), ('2099-02-02', 0, 260), ('2099-03-01', 0, 149)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .execute(pool)
            .await
            .unwrap_or_else(|error| {
                panic!(
                    "種入合計市值失敗：{error}
{sql}"
                )
            });
    }
    let (status, json) = get("/api/v1/portfolio/history?member_id=0").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["return_percent"], -4.23);
    assert_eq!(json["buys"], 160.0);

    for query in ["member_id=-1", "months=0", "months=121"] {
        let (status, _) = get(&format!("/api/v1/portfolio/history?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    cleanup_history().await;
}

/// 股利現金流預估：已公告的配息直接列入、去年同期推一年、全年彙總列不計，
/// 以目前持股股數計算金額與預估殖利率。預估以今天為基準，配息日期只能由今天推算；
/// 列屬於假代號與不存在的會員，結束時清除。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_dividend_forecast_endpoint_db_semantics() {
    use chrono::{Datelike, Days};

    const MEMBER: i64 = 979_790;
    let Some(pool) = pool_or_skip("portfolio_dividend_forecast_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_forecast = || async {
        let _ = sqlx::query("DELETE FROM stock_ownership_details WHERE member_id = $1")
            .bind(MEMBER)
            .execute(pool)
            .await;
        cleanup(pool).await;
    };
    cleanup_forecast().await;

    let path = format!("/api/v1/portfolio/dividend-forecast?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total"], 0.0);
    assert_eq!(json["items"], serde_json::json!([]));
    assert_eq!(json["months"].as_array().map(Vec::len), Some(13));

    seed(pool, &[SEED_STOCK]).await;
    let today = chrono::Local::now().date_naive();
    let fmt = |date: chrono::NaiveDate| date.format("%Y-%m-%d").to_string();
    let announced_ex = today + Days::new(20);
    let past_ex = today - Days::new(100);
    sqlx::query(
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost) VALUES ($1, $2, 1000, -50000)",
    )
    .bind(MEMBER)
    .bind(SYMBOL)
    .execute(pool)
    .await
    .expect("種入持股");
    sqlx::query("INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ('2026-04-30', $1, 100)")
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("種入報價");
    for (year, year_of_dividend, quarter, cash, ex, payable) in [
        (
            today.year(),
            today.year(),
            "Q4",
            2,
            fmt(announced_ex),
            fmt(announced_ex + Days::new(20)),
        ),
        (
            past_ex.year(),
            past_ex.year(),
            "Q1",
            1,
            fmt(past_ex),
            fmt(past_ex + Days::new(20)),
        ),
        (
            today.year(),
            today.year() - 1,
            "",
            10,
            "-".to_owned(),
            "-".to_owned(),
        ),
    ] {
        sqlx::query(r#"INSERT INTO dividend (security_code, year, year_of_dividend, quarter, cash_dividend, stock_dividend, "sum", "ex-dividend_date1", "ex-dividend_date2", payable_date1, payable_date2) VALUES ($1, $2, $3, $4, $5, 0, $5, $6, '-', $7, '-')"#)
            .bind(SYMBOL)
            .bind(year)
            .bind(year_of_dividend)
            .bind(quarter)
            .bind(Decimal::from(cash))
            .bind(ex)
            .bind(payable)
            .execute(pool)
            .await
            .expect("種入 dividend");
    }

    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 已公告 2 元 × 1,000 股＋去年同期 1 元 × 1,000 股；全年彙總列不計。
    assert_eq!(
        (
            json["announced_total"].clone(),
            json["estimated_total"].clone(),
            json["total"].clone()
        ),
        (2000.0.into(), 1000.0.into(), 3000.0.into())
    );
    assert_eq!(json["forward_yield_percent"], 3.0);
    let items = json["items"].as_array().expect("items 陣列");
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["basis"], "announced");
    assert_eq!(items[0]["ex_date"], fmt(announced_ex));
    assert_eq!(items[1]["basis"], "estimated");

    let (_, json) = get(&format!("{path}&months=1")).await;
    assert_eq!(json["estimated_total"], 0.0, "一個月內只有已公告的那筆");

    for query in ["member_id=-1", "months=0", "months=25"] {
        let (status, _) = get(&format!("/api/v1/portfolio/dividend-forecast?{query}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{query}");
    }

    cleanup_forecast().await;
}
