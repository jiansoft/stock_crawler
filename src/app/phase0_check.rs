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
