//! 持股通知用的查詢。
//!
//! 這裡的查詢只服務 Telegram 的月營收與財報通知，且一律只看**目前持有**的股票
//! （`stock_ownership_details.is_sold = false`）。全市場每個月有上千檔公布營收，
//! 全推等於把通知變成雜訊；持股只有數十檔，才有全部列出來的空間。
//!
//! 兩個查詢都是唯讀，不會寫入任何資料。

use anyhow::Result;
use sqlx::{Row, postgres::PgRow};

use crate::domain::financial::entity::{HoldingFinancialAlert, HoldingRevenueAlert};
use crate::infra::database;

// 兩個查詢都以 `security_code IN (SELECT ... WHERE is_sold = false)` 限定持股。
// 這段條件刻意寫在各自的字面 SQL 裡而不是用 format! 組出來：
// sqlx 0.9 只接受 &'static str（避免動態 SQL 注入），而這裡本來就沒有動態成分。

/// 取得指定月份**所有持股**的月營收，依年增率由大到小排序。
///
/// `date` 為營收月份，格式 yyyyMM（與 `"Revenue"."Date"` 一致）。
///
/// 一併帶出推估 EPS 需要的兩組資料：
///
/// 1. **錨點**（優先）：今年自 Q1 起連續公布、且季底不晚於本期的季報 EPS 加總，
///    以及該季底當月的累計營收。有了這組數字就能「已實現的用財報、未公布的按營收外推」。
///    要求季別連續（`COUNT(*) = MAX(季別編號)`）：缺季時加總會少算，寧可放棄錨點。
/// 2. **近四季平均稅後淨利率**（備援）：今年還沒有任何季報可錨定時才用得到。台股季報的
///    公布期限比月營收晚（Q1 約 5 月中、Q2 約 8 月中），所以 1~4 月的營收通知通常只有備援可用。
///    只取 Q1~Q4，排除年度列以免同一段期間被重複計入；排除 `net_income = 0`，
///    那是「尚未回補」的預設值而不是真的零利潤。
///
/// 兩組都查不到時對應欄位為 NULL，由呼叫端決定不顯示推估值。
///
/// 季別編號一律用 `CASE` 對照而不是 `CAST(SUBSTRING(quarter, 2, 1) AS int)`：年度財報的
/// `quarter` 實際存的是空字串（schema 註解寫 `'A'`，但正式庫有兩萬多列是 `''`），
/// 對它做 CAST 會丟 `22P02 invalid input syntax for type integer`。WHERE 子句的求值順序
/// 不保證，不能指望 `quarter IN ('Q1'..'Q4')` 一定先擋掉——換成 `CASE` 就沒有這個例外，
/// 非季別的值會落成 NULL 而被自然濾掉。
///
/// 只有當月有營收紀錄的持股會出現在結果中——ETF 與不公布月營收的個股本來就無從估算。
///
/// # Errors
/// 當 SQL 執行失敗時回傳錯誤。
pub async fn fetch_holding_revenue_alerts(date: i64) -> Result<Vec<HoldingRevenueAlert>> {
    let alerts = sqlx::query(
        r#"
SELECT
    r."stock_symbol",
    s."Name" AS stock_name,
    si.name AS industry_name,
    s.issued_share,
    r."Date",
    r."Monthly",
    r."MonthlyAccumulated",
    r."ComparedWithLastMonth",
    r."ComparedWithLastYearSameMonth",
    r."AccumulatedComparedWithLastYear",
    margin.net_income_margin,
    anchor.anchor_eps,
    anchor.anchor_accumulated_revenue
FROM "Revenue" AS r
INNER JOIN stocks AS s ON s.stock_symbol = r."stock_symbol"
LEFT JOIN stock_industry AS si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN LATERAL (
    SELECT
        reported.eps_sum AS anchor_eps,
        anchor_revenue."MonthlyAccumulated" AS anchor_accumulated_revenue
    FROM (
        SELECT
            MAX(CASE fs.quarter
                    WHEN 'Q1' THEN 1 WHEN 'Q2' THEN 2
                    WHEN 'Q3' THEN 3 WHEN 'Q4' THEN 4
                END) AS quarter_no,
            COUNT(*) AS quarter_count,
            SUM(fs.earnings_per_share) AS eps_sum
        FROM financial_statement AS fs
        WHERE fs.security_code = r."stock_symbol"
            AND fs.year = r."Date" / 100
            AND fs.quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
            AND CASE fs.quarter
                    WHEN 'Q1' THEN 3 WHEN 'Q2' THEN 6
                    WHEN 'Q3' THEN 9 WHEN 'Q4' THEN 12
                END <= r."Date" % 100
    ) AS reported
    INNER JOIN "Revenue" AS anchor_revenue
        ON anchor_revenue."stock_symbol" = r."stock_symbol"
        AND anchor_revenue."Date" = (r."Date" / 100) * 100 + reported.quarter_no * 3
    WHERE reported.quarter_count = reported.quarter_no
) AS anchor ON TRUE
LEFT JOIN LATERAL (
    SELECT AVG(recent.net_income) AS net_income_margin
    FROM (
        SELECT fs.net_income
        FROM financial_statement AS fs
        WHERE fs.security_code = r."stock_symbol"
            AND fs.quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
            AND fs.net_income <> 0
        ORDER BY fs.year DESC, fs.quarter DESC
        LIMIT 4
    ) AS recent
) AS margin ON TRUE
WHERE r."Date" = $1
    AND r."stock_symbol" IN (
        SELECT security_code FROM stock_ownership_details WHERE is_sold = false
    )
ORDER BY r."ComparedWithLastYearSameMonth" DESC, r."stock_symbol"
        "#,
    )
    .bind(date)
    .try_map(|row: PgRow| {
        Ok(HoldingRevenueAlert {
            stock_symbol: row.try_get("stock_symbol")?,
            stock_name: row.try_get("stock_name")?,
            industry_name: row.try_get("industry_name")?,
            date: row.try_get("Date")?,
            monthly: row.try_get("Monthly")?,
            monthly_accumulated: row.try_get("MonthlyAccumulated")?,
            compared_with_last_month: row.try_get("ComparedWithLastMonth")?,
            compared_with_last_year_same_month: row.try_get("ComparedWithLastYearSameMonth")?,
            accumulated_compared_with_last_year: row.try_get("AccumulatedComparedWithLastYear")?,
            issued_share: row.try_get("issued_share")?,
            net_income_margin: row.try_get("net_income_margin")?,
            anchor_eps: row.try_get("anchor_eps")?,
            anchor_accumulated_revenue: row.try_get("anchor_accumulated_revenue")?,
        })
    })
    .fetch_all(database::get_connection())
    .await?;

    Ok(alerts)
}

/// 取得指定年度、季度的持股季報，並帶出去年同季的 EPS 供比較。
///
/// 去年同季以 LEFT JOIN 取得：新上市或財報尚未回補完整時查不到，
/// 此時 `last_year_earnings_per_share` 為 `None`，通知會略過比較而不是顯示 0。
///
/// `year` 一定要在 SQL 裡 `CAST(... AS int)`：`financial_statement.year` 是 `bigint`，
/// 而 `HoldingFinancialAlert.year` 是 `i32`，直接取會在解碼時炸掉
/// （`mismatched types; Rust type i32 (as SQL type INT4) is not compatible with SQL type INT8`），
/// 整則持股季報通知會靜靜地失敗——正式站的 error log 每季 04:00 都會出現一次。
///
/// # Errors
/// 當 SQL 執行失敗時回傳錯誤。
pub async fn fetch_holding_financial_alerts(
    year: i32,
    quarter: &str,
) -> Result<Vec<HoldingFinancialAlert>> {
    let alerts = sqlx::query(
        r#"
SELECT
    fs.security_code AS stock_symbol,
    s."Name" AS stock_name,
    CAST(fs.year AS int) AS year,
    fs.quarter,
    fs.earnings_per_share,
    fs.return_on_equity,
    fs.gross_profit,
    prev.earnings_per_share AS last_year_earnings_per_share
FROM financial_statement AS fs
INNER JOIN stocks AS s ON s.stock_symbol = fs.security_code
LEFT JOIN financial_statement AS prev
    ON prev.security_code = fs.security_code
    AND prev.year = fs.year - 1
    AND prev.quarter = fs.quarter
WHERE fs.year = $1
    AND fs.quarter = $2
    AND fs.security_code IN (
        SELECT security_code FROM stock_ownership_details WHERE is_sold = false
    )
ORDER BY fs.security_code
        "#,
    )
    .bind(year)
    .bind(quarter)
    .try_map(|row: PgRow| {
        Ok(HoldingFinancialAlert {
            stock_symbol: row.try_get("stock_symbol")?,
            stock_name: row.try_get("stock_name")?,
            year: row.try_get("year")?,
            quarter: row.try_get("quarter")?,
            earnings_per_share: row.try_get("earnings_per_share")?,
            return_on_equity: row.try_get("return_on_equity")?,
            gross_profit: row.try_get("gross_profit")?,
            last_year_earnings_per_share: row.try_get("last_year_earnings_per_share")?,
        })
    })
    .fetch_all(database::get_connection())
    .await?;

    Ok(alerts)
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;
    use rust_decimal_macros::dec;

    use crate::domain::financial::entity::EpsEstimateBasis;

    use super::*;

    /// 測試用假代號；正式資料不會用到 7997x 這段號碼。
    /// A：有完整錨點；B：什麼都沒有；C：只有淨利率可備援；D：季別不連續。
    const FAKE_SYMBOL_A: &str = "79979";
    const FAKE_SYMBOL_B: &str = "79978";
    const FAKE_SYMBOL_C: &str = "79977";
    const FAKE_SYMBOL_D: &str = "79976";

    /// 測試用營收月份，固定歷史月份而非 `Local::now()`，避免與真實資料撞期。
    const TEST_DATE: i64 = 202608;
    /// 錨點季底（2026 Q2）對應的營收月份。
    const ANCHOR_DATE: i64 = 202606;

    /// `stock_industry.sql` 已預先建立編號 1（水泥工業）。
    const TEST_INDUSTRY: i32 = 1;

    fn fake_symbols() -> Vec<String> {
        [FAKE_SYMBOL_A, FAKE_SYMBOL_B, FAKE_SYMBOL_C, FAKE_SYMBOL_D]
            .iter()
            .map(|symbol| symbol.to_string())
            .collect()
    }

    /// 移除測試寫入的所有資料列；測試前後都要呼叫，避免污染資料庫。
    async fn cleanup() {
        let symbols = fake_symbols();
        let conn = database::get_connection();

        for sql in [
            r#"DELETE FROM "Revenue" WHERE stock_symbol = ANY($1)"#,
            "DELETE FROM financial_statement WHERE security_code = ANY($1)",
            "DELETE FROM stock_ownership_details WHERE security_code = ANY($1)",
            "DELETE FROM stocks WHERE stock_symbol = ANY($1)",
        ] {
            let _ = sqlx::query(sql).bind(&symbols).execute(conn).await;
        }
    }

    async fn seed_stock(symbol: &str, name: &str, industry: i32, issued_share: i64) {
        let conn = database::get_connection();

        let _ = sqlx::query(
            r#"INSERT INTO stocks ("SecurityCode", "Name", stock_symbol, stock_industry_id, issued_share)
               VALUES ($1, $2, $1, $3, $4)"#,
        )
        .bind(symbol)
        .bind(name)
        .bind(industry)
        .bind(issued_share)
        .execute(conn)
        .await;

        let _ = sqlx::query(
            "INSERT INTO stock_ownership_details (security_code, share_quantity, is_sold)
             VALUES ($1, 1000, false)",
        )
        .bind(symbol)
        .execute(conn)
        .await;
    }

    async fn seed_revenue(symbol: &str, date: i64, accumulated: Decimal, yoy: Decimal) {
        let _ = sqlx::query(
            r#"INSERT INTO "Revenue"
               ("SecurityCode", stock_symbol, "Date", "Monthly", "MonthlyAccumulated",
                "ComparedWithLastMonth", "ComparedWithLastYearSameMonth",
                "AccumulatedComparedWithLastYear")
               VALUES ($1, $1, $2, 150000, $3, 5, $4, 12.5)"#,
        )
        .bind(symbol)
        .bind(date)
        .bind(accumulated)
        .bind(yoy)
        .execute(database::get_connection())
        .await;
    }

    async fn seed_financial(
        symbol: &str,
        year: i64,
        quarter: &str,
        net_income: Decimal,
        eps: Decimal,
    ) {
        let _ = sqlx::query(
            "INSERT INTO financial_statement
             (security_code, year, quarter, net_income, earnings_per_share)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(symbol)
        .bind(year)
        .bind(quarter)
        .bind(net_income)
        .bind(eps)
        .execute(database::get_connection())
        .await;
    }

    async fn seed() {
        // A：產業 1、發行股數 1 億股，今年 Q1+Q2 連續公布且有 6 月營收 ⇒ 走錨定法。
        seed_stock(FAKE_SYMBOL_A, "測試甲", TEST_INDUSTRY, 100_000_000).await;
        seed_revenue(FAKE_SYMBOL_A, ANCHOR_DATE, dec!(600000), dec!(8)).await;
        seed_revenue(FAKE_SYMBOL_A, TEST_DATE, dec!(1000000), dec!(10)).await;
        seed_financial(FAKE_SYMBOL_A, 2026, "Q1", dec!(40), dec!(1.2)).await;
        seed_financial(FAKE_SYMBOL_A, 2026, "Q2", dec!(40), dec!(1.8)).await;
        // Q3 的季底（9 月）晚於本期（8 月），不能被當成錨點——否則會拿到還沒公布的營收期間。
        seed_financial(FAKE_SYMBOL_A, 2026, "Q3", dec!(40), dec!(99)).await;
        // 年度列與更舊的季度都不該影響結果。年度列的 quarter 在正式庫存的是**空字串**
        // （schema 註解寫 'A'，實際資料兩者都有），對它做 CAST 會丟 22P02，
        // 因此兩種寫法都放進來，確保季別編號的對照不會在任一種上炸掉。
        seed_financial(FAKE_SYMBOL_A, 2026, "", dec!(900), dec!(99)).await;
        seed_financial(FAKE_SYMBOL_A, 2026, "A", dec!(900), dec!(99)).await;
        seed_financial(FAKE_SYMBOL_A, 2025, "Q4", dec!(-500), dec!(99)).await;

        // B：stock_industry_id = 0 對不到分類，發行股數 0、淨利率 0 ⇒ 兩條路都走不通。
        seed_stock(FAKE_SYMBOL_B, "測試乙", 0, 0).await;
        seed_revenue(FAKE_SYMBOL_B, TEST_DATE, dec!(1000000), dec!(50)).await;
        seed_financial(FAKE_SYMBOL_B, 2026, "Q2", dec!(0), dec!(0)).await;

        // C：今年沒有任何季報，只有去年四季的淨利率 ⇒ 退回淨利率法。
        seed_stock(FAKE_SYMBOL_C, "測試丙", TEST_INDUSTRY, 100_000_000).await;
        seed_revenue(FAKE_SYMBOL_C, TEST_DATE, dec!(1000000), dec!(30)).await;
        for quarter in ["Q1", "Q2", "Q3", "Q4"] {
            seed_financial(FAKE_SYMBOL_C, 2025, quarter, dec!(40), dec!(1)).await;
        }

        // D：今年只有 Q2、缺 Q1，季別不連續 ⇒ 放棄錨點；淨利率也是 0 ⇒ 不推估。
        seed_stock(FAKE_SYMBOL_D, "測試丁", TEST_INDUSTRY, 100_000_000).await;
        seed_revenue(FAKE_SYMBOL_D, ANCHOR_DATE, dec!(600000), dec!(20)).await;
        seed_revenue(FAKE_SYMBOL_D, TEST_DATE, dec!(1000000), dec!(20)).await;
        seed_financial(FAKE_SYMBOL_D, 2026, "Q2", dec!(0), dec!(1.8)).await;
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch_holding_revenue_alerts_returns_all_holdings_sorted_by_yoy() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_fetch_holding_revenue_alerts：無資料庫連接");
            return;
        }

        cleanup().await;
        seed().await;

        let alerts = fetch_holding_revenue_alerts(TEST_DATE)
            .await
            .expect("fetch_holding_revenue_alerts");
        let fetched: Vec<_> = alerts
            .into_iter()
            .filter(|alert| fake_symbols().contains(&alert.stock_symbol))
            .collect();

        assert_eq!(fetched.len(), 4, "四檔持股都要出現，不再有年增率門檻");

        // 年增率高的排前面：B(50) > C(30) > D(20) > A(10)。
        let order: Vec<&str> = fetched
            .iter()
            .map(|alert| alert.stock_symbol.as_str())
            .collect();
        assert_eq!(
            order,
            vec![FAKE_SYMBOL_B, FAKE_SYMBOL_C, FAKE_SYMBOL_D, FAKE_SYMBOL_A]
        );

        let find = |symbol: &str| {
            fetched
                .iter()
                .find(|alert| alert.stock_symbol == symbol)
                .expect("symbol")
        };

        // A：錨點 EPS 1.2 + 1.8 ＝ 3 元，比率 1,000,000 ÷ 600,000 ⇒ 5 元。
        // 若 Q3（季底晚於本期）、全年度列或去年季度被誤納入，數字就不會是 5。
        let a = find(FAKE_SYMBOL_A);
        assert_eq!(a.stock_name, "測試甲");
        assert_eq!(a.industry_name.as_deref(), Some("水泥工業"));
        assert_eq!(a.monthly_accumulated, dec!(1000000));
        assert_eq!(a.accumulated_compared_with_last_year, dec!(12.5));
        assert_eq!(a.issued_share, 100_000_000);
        assert_eq!(a.anchor_eps, Some(dec!(3)));
        assert_eq!(a.anchor_accumulated_revenue, Some(dec!(600000)));
        let estimate = a.estimate_eps().expect("A 應該推估得出來");
        assert_eq!(estimate.basis, EpsEstimateBasis::ReportedQuarters);
        assert_eq!(estimate.accumulated, dec!(5));

        // B：查無產業、無錨點、淨利率 0（尚未回補的預設值）⇒ 不推估。
        let b = find(FAKE_SYMBOL_B);
        assert_eq!(b.industry_name, None, "查無產業分類時應為 None");
        assert_eq!(b.anchor_eps, None);
        assert_eq!(b.net_income_margin, None, "net_income = 0 的列要被排除");
        assert_eq!(b.estimate_eps(), None);

        // C：今年沒有季報 ⇒ 退回淨利率法，10 億元 × 40% ÷ 1 億股 ＝ 4 元。
        let c = find(FAKE_SYMBOL_C);
        assert_eq!(c.anchor_eps, None, "去年的季報不能當今年的錨點");
        assert_eq!(
            c.net_income_margin.map(|margin| margin.round_dp(4)),
            Some(dec!(40))
        );
        let estimate = c.estimate_eps().expect("C 應該退回淨利率法");
        assert_eq!(estimate.basis, EpsEstimateBasis::NetIncomeMargin);
        assert_eq!(estimate.accumulated, dec!(4));

        // D：缺 Q1，季別不連續 ⇒ 放棄錨點（否則會少算 Q1 而低估）。
        let d = find(FAKE_SYMBOL_D);
        assert_eq!(d.anchor_eps, None, "季別不連續時不能當錨點");
        assert_eq!(d.estimate_eps(), None);

        cleanup().await;
    }

    /// 季報通知的回歸測試：`financial_statement.year` 是 `bigint`，
    /// `HoldingFinancialAlert.year` 是 `i32`，少了 SQL 的 CAST 就會在解碼時整個查詢失敗。
    ///
    /// 這個 bug 在正式站存在過，症狀是每季 04:00 的持股季報通知靜靜地不送、
    /// 只在 error log 留下 `mismatched types ... INT4 ... INT8`。
    /// 斷言的重點是「查詢不回錯」，而不是回了幾筆。
    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_fetch_holding_financial_alerts_decodes_bigint_year() {
        dotenvy::dotenv().ok();
        if database::ping().await.is_err() {
            println!("跳過 test_fetch_holding_financial_alerts_decodes_bigint_year：無資料庫連接");
            return;
        }

        cleanup().await;
        seed().await;

        let alerts = fetch_holding_financial_alerts(2026, "Q2")
            .await
            .expect("year 是 bigint，沒有 CAST 會在這裡炸掉");

        let a = alerts
            .iter()
            .find(|alert| alert.stock_symbol == FAKE_SYMBOL_A)
            .expect("測試甲的 2026 Q2 財報應該被查到");
        assert_eq!(a.year, 2026);
        assert_eq!(a.quarter, "Q2");
        assert_eq!(a.earnings_per_share, dec!(1.8));

        cleanup().await;
    }
}
