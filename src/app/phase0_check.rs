//! Phase 0 資料品質確認（暫時模組，完成後移除）。
//!
//! 對應 `docs/stock-mcp-expanded-tools-plan.md` §10 的 P0-1～P0-4：
//! 以 `#[ignore]` 測試直連資料庫執行一次性檢查，結果由執行者記回計畫文件。

#[cfg(test)]
mod tests {
    use sqlx::Row;

    use crate::infra::database;

    /// P0-1～P0-4 一次執行：印出各項檢查結果供人工記錄。
    #[tokio::test]
    #[ignore = "Phase 0 一次性資料品質檢查，需資料庫連線，手動執行"]
    async fn phase0_data_quality_check() {
        dotenvy::dotenv().ok();
        let pool = database::get_connection();

        // P0-1：Revenue.Date 是否全為六位 YYYYMM（月份 01-12）。
        let bad_dates = sqlx::query(
            r#"SELECT COUNT(*) AS cnt FROM "Revenue"
               WHERE "Date" < 100001 OR "Date" > 999912 OR "Date" % 100 NOT BETWEEN 1 AND 12"#,
        )
        .fetch_one(pool)
        .await
        .expect("P0-1 query failed");
        let min_max = sqlx::query(
            r#"SELECT MIN("Date") AS mn, MAX("Date") AS mx, COUNT(*) AS total FROM "Revenue""#,
        )
        .fetch_one(pool)
        .await
        .expect("P0-1 min/max failed");
        println!(
            "P0-1 Revenue.Date: 異常筆數={}, min={:?}, max={:?}, 總筆數={:?}",
            bad_dates.get::<i64, _>("cnt"),
            min_max.get::<Option<i64>, _>("mn"),
            min_max.get::<Option<i64>, _>("mx"),
            min_max.get::<i64, _>("total"),
        );

        // P0-2：quarter 實際值域（sqlx 0.9 要求字面 SQL，兩張表分開列）。
        for (table, sql) in [
            (
                "financial_statement",
                "SELECT quarter, COUNT(*) AS cnt FROM financial_statement GROUP BY quarter ORDER BY quarter",
            ),
            (
                "dividend",
                "SELECT quarter, COUNT(*) AS cnt FROM dividend GROUP BY quarter ORDER BY quarter",
            ),
        ] {
            let rows = sqlx::query(sql)
                .fetch_all(pool)
                .await
                .expect("P0-2 query failed");
            let summary: Vec<String> = rows
                .iter()
                .map(|r| {
                    format!(
                        "{:?}={}",
                        r.get::<String, _>("quarter"),
                        r.get::<i64, _>("cnt")
                    )
                })
                .collect();
            println!("P0-2 {table}.quarter 值域: {}", summary.join(", "));
        }

        // P0-3：股利日期欄位的非合法日期標記統計（四個欄位分開列字面 SQL）。
        for (col, sql) in [
            (
                "ex-dividend_date1",
                r#"SELECT "ex-dividend_date1" AS v, COUNT(*) AS cnt FROM dividend
                   WHERE "ex-dividend_date1" !~ '^\d{4}-\d{2}-\d{2}$'
                   GROUP BY "ex-dividend_date1" ORDER BY cnt DESC LIMIT 10"#,
            ),
            (
                "ex-dividend_date2",
                r#"SELECT "ex-dividend_date2" AS v, COUNT(*) AS cnt FROM dividend
                   WHERE "ex-dividend_date2" !~ '^\d{4}-\d{2}-\d{2}$'
                   GROUP BY "ex-dividend_date2" ORDER BY cnt DESC LIMIT 10"#,
            ),
            (
                "payable_date1",
                r#"SELECT payable_date1 AS v, COUNT(*) AS cnt FROM dividend
                   WHERE payable_date1 !~ '^\d{4}-\d{2}-\d{2}$'
                   GROUP BY payable_date1 ORDER BY cnt DESC LIMIT 10"#,
            ),
            (
                "payable_date2",
                r#"SELECT payable_date2 AS v, COUNT(*) AS cnt FROM dividend
                   WHERE payable_date2 !~ '^\d{4}-\d{2}-\d{2}$'
                   GROUP BY payable_date2 ORDER BY cnt DESC LIMIT 10"#,
            ),
        ] {
            let rows = sqlx::query(sql)
                .fetch_all(pool)
                .await
                .expect("P0-3 query failed");
            let summary: Vec<String> = rows
                .iter()
                .map(|r| format!("{:?}={}", r.get::<String, _>("v"), r.get::<i64, _>("cnt")))
                .collect();
            println!("P0-3 dividend.{col} 非日期值: {}", summary.join(", "));
        }

        // P0-4（Phase 4）：三個市場輔助查詢的執行計畫。
        // SQL 形狀與 data_api 實作一致（相同 WHERE／排序／LIMIT），僅把
        // 綁定參數改為字面值，方便一次性人工執行與記錄。
        for (name, sql) in [
            (
                "market/index-history",
                // §4.8：固定 TAIEX、日期區間、date DESC LIMIT 30。
                // `index-date_category-uidx` 以 (date, category) 為鍵，
                // 預期走索引反向掃描。
                r#"EXPLAIN (ANALYZE, BUFFERS)
                   SELECT "date", index, change, trade_value, "transaction", trading_volume
                   FROM index
                   WHERE category = 'TAIEX'
                     AND "date" >= '2026-01-01' AND "date" <= '2026-07-17'
                   ORDER BY "date" DESC LIMIT 30"#,
            ),
            (
                "market/dividend-calendar",
                // §4.9：四個字串日期欄位的 UNION ALL 行事曆掃描。字串欄位
                // 含 `-`、`尚未公布` 等髒資料，必須以 CASE + regex 先過濾
                // 再轉 date，無法利用索引做範圍查詢——此 EXPLAIN 用來確認
                // 全表掃描成本是否可接受（dividend 每年僅數千筆）。
                r#"EXPLAIN (ANALYZE, BUFFERS)
                   SELECT stock_symbol, event_type, event_date
                   FROM (
                       SELECT d.security_code AS stock_symbol, 'ex_dividend' AS event_type,
                              CASE WHEN d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$'
                                   THEN d."ex-dividend_date1"::date END AS event_date
                       FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
                       UNION ALL
                       SELECT d.security_code, 'ex_rights',
                              CASE WHEN d."ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$'
                                   THEN d."ex-dividend_date2"::date END
                       FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
                       UNION ALL
                       SELECT d.security_code, 'cash_payable',
                              CASE WHEN d.payable_date1 ~ '^\d{4}-\d{2}-\d{2}$'
                                   THEN d.payable_date1::date END
                       FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
                       UNION ALL
                       SELECT d.security_code, 'stock_payable',
                              CASE WHEN d.payable_date2 ~ '^\d{4}-\d{2}-\d{2}$'
                                   THEN d.payable_date2::date END
                       FROM dividend d JOIN stocks s ON s.stock_symbol = d.security_code
                   ) events
                   WHERE event_date BETWEEN '2026-07-01' AND '2026-07-31'
                   ORDER BY event_date ASC, stock_symbol ASC LIMIT 50"#,
            ),
            (
                "market/qfii-holding-ranking",
                // §4.10：stocks 表快照排行。stocks 僅數千列，即使 Seq Scan
                // + top-N sort 也應在毫秒級；此處確認實際成本。
                r#"EXPLAIN (ANALYZE, BUFFERS)
                   SELECT stock_symbol, qfii_shares_held, qfii_share_holding_percentage
                   FROM stocks
                   WHERE stock_exchange_market_id IN (2, 4)
                     AND "SuspendListing" = false
                     AND qfii_shares_held <> 0
                   ORDER BY qfii_share_holding_percentage DESC, stock_symbol ASC LIMIT 20"#,
            ),
        ] {
            let rows = sqlx::query(sql)
                .fetch_all(pool)
                .await
                .expect("P0-4 phase4 explain failed");
            println!("P0-4 [{name}] 執行計畫:");
            for r in rows {
                println!("  {}", r.get::<String, _>(0));
            }
        }

        // P0-4：Phase 1 三類查詢的執行計畫（各表以 2330 為代表）。
        for (name, sql) in [
            (
                "monthly-revenues",
                r#"EXPLAIN (ANALYZE, BUFFERS)
                   SELECT * FROM "Revenue" WHERE "SecurityCode" = '2330'
                   ORDER BY "Date" DESC LIMIT 24"#,
            ),
            (
                "financial-statements",
                r#"EXPLAIN (ANALYZE, BUFFERS)
                   SELECT * FROM financial_statement WHERE security_code = '2330'
                   AND quarter IN ('Q1','Q2','Q3','Q4')
                   ORDER BY "year" DESC, quarter DESC LIMIT 12"#,
            ),
            (
                "dividends",
                r#"EXPLAIN (ANALYZE, BUFFERS)
                   SELECT * FROM dividend WHERE security_code = '2330'
                   ORDER BY year_of_dividend DESC LIMIT 20"#,
            ),
        ] {
            let rows = sqlx::query(sql)
                .fetch_all(pool)
                .await
                .expect("P0-4 explain failed");
            println!("P0-4 [{name}] 執行計畫:");
            for r in rows {
                println!("  {}", r.get::<String, _>(0));
            }
        }
    }
}
