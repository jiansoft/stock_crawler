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
        r#"INSERT INTO last_daily_quotes (date, stock_symbol, closing_price, price_earning_ratio, "price-to-book_ratio") VALUES ('2026-04-30', $2, 50, 12.5, 0)"#,
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
    // 營收：上個月起連 3 個月年減（−5、−3、−1），第 4 個月年增；財報：上一季營業虧損且三率都比去年同季低。
    let month_ord = today.year() * 12 + today.month0() as i32;
    for (back, yoy) in [(1, -5), (2, -3), (3, -1), (4, 2)] {
        let ord = month_ord - back;
        sqlx::query(r#"INSERT INTO "Revenue" ("SecurityCode", stock_symbol, "Date", "ComparedWithLastYearSameMonth") VALUES ($1, $1, $2, $3)"#)
            .bind(SYMBOL)
            .bind(i64::from((ord / 12) * 100 + ord % 12 + 1))
            .bind(Decimal::from(yoy))
            .execute(pool)
            .await
            .expect("種入月營收");
    }
    let quarter_ord = today.year() * 4 + (today.month0() / 3) as i32 - 1;
    let (margin_year, margin_quarter) = (quarter_ord / 4, quarter_ord % 4 + 1);
    sqlx::query(
        "INSERT INTO income_statement (stock_symbol, fiscal_year, period_type, quarter, source, revenue, gross_profit, operating_profit, net_income)          VALUES ($1, $2, 'single', $3, 'yahoo', 100, 30, -2, 1), ($1, $2 - 1, 'single', $3, 'yahoo', 100, 35, 5, 3)",
    )
    .bind(SYMBOL)
    .bind(margin_year)
    .bind(format!("Q{margin_quarter}"))
    .execute(pool)
    .await
    .expect("種入損益表");
    // 財務體質：上一季的近四季自由現金流 −300 千元。
    sqlx::query("INSERT INTO financial_health (stock_symbol, f_score_year, f_score, f_score_evaluated, quarter_year, quarter, free_cash_flow_ttm) VALUES ($1, $2, 6, 9, $3, $4, -300)")
        .bind(SYMBOL)
        .bind(today.year() - 1)
        .bind(margin_year)
        .bind(i16::try_from(margin_quarter).expect("季別 1–4"))
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
    // 本益比 12.5；淨值比 0 視為沒有資料。
    assert_eq!(json["weighted_pe"]["value"], 12.5);
    assert!(json["weighted_pb"].is_null(), "{json}");
    let holding = &json["holdings"][0];
    assert_eq!(holding["stock_symbol"], SYMBOL);
    assert_eq!(
        (holding["shares"].clone(), holding["weight_percent"].clone()),
        (2_000.into(), 100.0.into())
    );
    assert_eq!(holding["f_score"], 6);
    assert_eq!(holding["pe_ratio"], 12.5);
    assert!(holding["pb_ratio"].is_null(), "{holding}");
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
    let last_month = month_ord - 1;
    assert_eq!(
        holding["revenue_month"],
        format!("{:04}-{:02}", last_month / 12, last_month % 12 + 1)
    );
    assert_eq!(holding["revenue_yoy_percent"], -5.0);
    assert_eq!(holding["revenue_decline_months"], 3);
    assert_eq!(
        holding["margin_period"],
        format!("{margin_year}-Q{margin_quarter}")
    );
    assert_eq!(
        (
            holding["gross_margin_percent"].clone(),
            holding["operating_margin_percent"].clone(),
            holding["net_margin_percent"].clone()
        ),
        (30.0.into(), (-2.0).into(), 1.0.into())
    );
    assert_eq!(holding["margins_trend"], "all_down");
    assert_eq!(holding["free_cash_flow_ttm"], -300);
    assert_eq!(
        holding["warnings"],
        serde_json::json!([
            "revenue_decline",
            "margins_all_down",
            "operating_loss",
            "negative_free_cash_flow"
        ])
    );
    assert_eq!(json["warning_symbols"], 1);
    // 金融保險業不取三率與自由現金流：只剩營收警示。
    sqlx::query("UPDATE stocks SET stock_industry_id = 17 WHERE stock_symbol = $1")
        .bind(SYMBOL)
        .execute(pool)
        .await
        .expect("改為金融保險業");
    let (_, json) = get(&path).await;
    let holding = &json["holdings"][0];
    assert!(holding["margin_period"].is_null(), "{holding}");
    assert!(holding["operating_margin_percent"].is_null(), "{holding}");
    assert!(holding["margins_trend"].is_null(), "{holding}");
    assert!(holding["free_cash_flow_ttm"].is_null(), "{holding}");
    assert_eq!(holding["warnings"], serde_json::json!(["revenue_decline"]));

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

/// 持股籌碼日報：日期格式錯誤回 422；查詢日之前沒有任何籌碼資料時回 200、`date` 為 null 與空清單。
/// 持股推算與籌碼欄位由倉儲的整合測試驗證。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_chip_endpoint_db_semantics() {
    if pool_or_skip("portfolio_chip_endpoint_db_semantics")
        .await
        .is_none()
    {
        return;
    }
    let (status, _) = get("/api/v1/portfolio/chip?date=2026-13-01").await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let (status, json) = get("/api/v1/portfolio/chip?date=1900-01-01").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert!(json["date"].is_null(), "{json}");
    assert_eq!(json["items"], serde_json::json!([]));
    assert_eq!(json["notable"], 0);
}

/// 持股填息：天數範圍檢查，回應帶回看天數、統計與清單（填息規則由 handler 與 domain 單元測試驗證）。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_dividend_fill_endpoint_db_semantics() {
    if pool_or_skip("portfolio_dividend_fill_endpoint_db_semantics")
        .await
        .is_none()
    {
        return;
    }
    for days in [29, 731] {
        let (status, _) = get(&format!("/api/v1/portfolio/dividend-fill?days={days}")).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "days={days}");
    }
    let (status, json) = get("/api/v1/portfolio/dividend-fill").await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["days"], 365);
    assert!(json["items"].is_array(), "{json}");
    assert!(json["summary"]["events"].is_number(), "{json}");
}

/// 持股穿透：持有的假 ETF 依前十大持股拆到成分股，與直接持有的假股票合併；
/// 沒有收盤價的部位列入 unpriced；member_id 篩選與 422。
#[tokio::test]
#[cfg_attr(
    not(feature = "integration-tests"),
    ignore = "需要外部服務（PostgreSQL），請加 --features integration-tests 執行"
)]
async fn portfolio_etf_exposure_endpoint_db_semantics() {
    const MEMBER: i64 = 979_791;
    const ETF: &str = "79979EX";
    const UNPRICED: &str = "79979UP";
    let Some(pool) = pool_or_skip("portfolio_etf_exposure_endpoint_db_semantics").await else {
        return;
    };
    let cleanup_exposure = || async {
        for sql in [
            "DELETE FROM stock_ownership_details WHERE member_id = $1",
            "DELETE FROM etf_holding WHERE stock_symbol = $2",
            "DELETE FROM etf_snapshot WHERE stock_symbol = $2",
            "DELETE FROM last_daily_quotes WHERE stock_symbol = $2",
        ] {
            let _ = sqlx::query(sql).bind(MEMBER).bind(ETF).execute(pool).await;
        }
        cleanup(pool).await;
    };
    cleanup_exposure().await;

    let path = format!("/api/v1/portfolio/etf-exposure?member_id={MEMBER}");
    let (status, json) = get(&path).await;
    assert_eq!(status, StatusCode::OK, "{json}");
    assert_eq!(json["total_value"], 0.0);
    assert_eq!(json["items"], serde_json::json!([]));

    seed(pool, &[SEED_STOCK]).await;
    for sql in [
        "INSERT INTO stock_ownership_details (member_id, security_code, share_quantity, holding_cost, is_sold) VALUES ($1, $2, 1000, -1, false), ($1, $3, 2000, -1, false), ($1, $4, 10, -1, false), ($1, $2, 500, -1, true)",
        "INSERT INTO last_daily_quotes (date, stock_symbol, closing_price) VALUES ('2026-04-30', $2, 50), ('2026-04-30', $3, 100)",
        "INSERT INTO etf_snapshot (stock_symbol, name, market_id) VALUES ($3, '端點測試 ETF', 2)",
        "INSERT INTO etf_holding (stock_symbol, rank, as_of, holding_symbol, holding_name, weight) VALUES ($3, 1, '2026-03-31', $2, '端點測試股', 25), ($3, 2, '2026-03-31', '', '現金', 5)",
    ] {
        sqlx::query(sql)
            .bind(MEMBER)
            .bind(SYMBOL)
            .bind(ETF)
            .bind(UNPRICED)
            .execute(pool)
            .await
            .unwrap_or_else(|error| panic!("種入持股失敗：{error}\n{sql}"));
    }

    let (status, json) = get(&format!("{path}&limit=1")).await;
    cleanup_exposure().await;
    assert_eq!(status, StatusCode::OK, "{json}");
    // 直接 1000 × 50 = 50,000；ETF 2000 × 100 = 200,000，25% 拆到同一檔、5% 為現金。
    assert_eq!(json["total_value"], 250_000.0);
    assert_eq!(json["etf_value"], 200_000.0);
    assert_eq!(json["uncovered_value"], 140_000.0);
    assert_eq!(json["total_items"], 2);
    assert_eq!(json["unpriced"], serde_json::json!([UNPRICED]));
    assert_eq!(json["etfs"][0]["name"], "端點測試 ETF");
    assert_eq!(json["etfs"][0]["holdings_as_of"], "2026-03-31");
    let items = json["items"].as_array().expect("items 是陣列");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["symbol"], SYMBOL);
    assert_eq!(items[0]["name"], "端點測試股");
    assert_eq!(items[0]["direct_value"], 50_000.0);
    assert_eq!(items[0]["etf_value"], 50_000.0);
    assert_eq!(items[0]["weight"], 40.0);
    assert_eq!(items[0]["sources"][0]["etf_symbol"], ETF);

    for bad in [
        "/api/v1/portfolio/etf-exposure?member_id=-1",
        "/api/v1/portfolio/etf-exposure?limit=0",
        "/api/v1/portfolio/etf-exposure?limit=201",
    ] {
        let (status, json) = get(bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{bad} {json}");
    }
}
