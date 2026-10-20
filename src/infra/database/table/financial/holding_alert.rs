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
///    另帶出錨點季往回共四季的 EPS 合計與同期十二個月營收合計（`ttm_*`），
///    錨點虧損時用來推估新增營收的淨利率；四季或十二個月不齊時為 NULL。
///    錨點虧損且有新增營收時，再帶出近 12 季「季 EPS 對季營收」的迴歸（`reg_*`），
///    營收一次入帳的公司（建設、生技）靠它才看得到「入帳一次賺多少」。
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
    anchor.anchor_accumulated_revenue,
    anchor.anchor_quarter_no,
    ttm.ttm_eps,
    ttm.ttm_revenue,
    reg.reg_slope,
    reg.reg_intercept,
    reg.reg_r2,
    reg.reg_quarters,
    record.previous_high,
    decline.yoy_decline_months
FROM "Revenue" AS r
INNER JOIN stocks AS s ON s.stock_symbol = r."stock_symbol"
LEFT JOIN stock_industry AS si ON si.stock_industry_id = s.stock_industry_id
LEFT JOIN LATERAL (
    SELECT
        reported.quarter_no AS anchor_quarter_no,
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
    -- 季別與月份都換成連續序號（年 × 4 + 季、年 × 12 + 月）才能跨年取區間。
    SELECT q.eps_sum AS ttm_eps, m.revenue_sum AS ttm_revenue
    FROM (
        SELECT COUNT(*) AS quarter_count, SUM(fs.earnings_per_share) AS eps_sum
        FROM financial_statement AS fs
        WHERE fs.security_code = r."stock_symbol"
            AND fs.quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
            AND fs.year * 4 + CASE fs.quarter
                    WHEN 'Q1' THEN 1 WHEN 'Q2' THEN 2
                    WHEN 'Q3' THEN 3 WHEN 'Q4' THEN 4
                END
                BETWEEN (r."Date" / 100) * 4 + anchor.anchor_quarter_no - 3
                    AND (r."Date" / 100) * 4 + anchor.anchor_quarter_no
    ) AS q
    CROSS JOIN (
        SELECT COUNT(*) AS month_count, SUM(rv."Monthly") AS revenue_sum
        FROM "Revenue" AS rv
        WHERE rv."stock_symbol" = r."stock_symbol"
            AND (rv."Date" / 100) * 12 + rv."Date" % 100
                BETWEEN (r."Date" / 100) * 12 + anchor.anchor_quarter_no * 3 - 11
                    AND (r."Date" / 100) * 12 + anchor.anchor_quarter_no * 3
    ) AS m
    WHERE q.quarter_count = 4 AND m.month_count = 12 AND m.revenue_sum > 0
) AS ttm ON TRUE
LEFT JOIN LATERAL (
    -- 每季營收要三個月齊全才算一個樣本；外層條件不成立時以 one-time filter 略過。
    SELECT
        REGR_SLOPE(qq.eps, qq.revenue)::numeric AS reg_slope,
        REGR_INTERCEPT(qq.eps, qq.revenue)::numeric AS reg_intercept,
        REGR_R2(qq.eps, qq.revenue)::numeric AS reg_r2,
        REGR_COUNT(qq.eps, qq.revenue) AS reg_quarters
    FROM (
        SELECT fs.earnings_per_share AS eps, SUM(rv."Monthly") AS revenue
        FROM financial_statement AS fs
        INNER JOIN "Revenue" AS rv
            ON rv."stock_symbol" = fs.security_code
            AND rv."Date" / 100 = fs.year
            AND (rv."Date" % 100 + 2) / 3 = CASE fs.quarter
                    WHEN 'Q1' THEN 1 WHEN 'Q2' THEN 2
                    WHEN 'Q3' THEN 3 WHEN 'Q4' THEN 4
                END
        WHERE fs.security_code = r."stock_symbol"
            AND fs.quarter IN ('Q1', 'Q2', 'Q3', 'Q4')
            AND fs.year * 4 + CASE fs.quarter
                    WHEN 'Q1' THEN 1 WHEN 'Q2' THEN 2
                    WHEN 'Q3' THEN 3 WHEN 'Q4' THEN 4
                END
                BETWEEN (r."Date" / 100) * 4 + anchor.anchor_quarter_no - 11
                    AND (r."Date" / 100) * 4 + anchor.anchor_quarter_no
        GROUP BY fs.year, fs.quarter, fs.earnings_per_share
        HAVING COUNT(*) = 3
    ) AS qq
    WHERE anchor.anchor_eps <= 0 AND r."MonthlyAccumulated" > anchor.anchor_accumulated_revenue
) AS reg ON TRUE
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
LEFT JOIN LATERAL (
    -- 之前所有月份的最高營收；之前不滿 12 個月不判斷是否創新高。
    SELECT CASE WHEN COUNT(*) >= 12 THEN MAX(h."Monthly") END AS previous_high
    FROM "Revenue" AS h
    WHERE h."stock_symbol" = r."stock_symbol" AND h."Date" < r."Date"
) AS record ON TRUE
LEFT JOIN LATERAL (
    -- 從當月往回、月份相連且年增率為負的月數（最多看 12 個月）。
    SELECT COALESCE(
               MIN(rn) FILTER (WHERE NOT (yoy < 0 AND ord = first_ord - rn + 1)) - 1,
               COUNT(*)
           ) AS yoy_decline_months
    FROM (
        SELECT yoy, ord,
            ROW_NUMBER() OVER (ORDER BY ord DESC) AS rn,
            FIRST_VALUE(ord) OVER (ORDER BY ord DESC) AS first_ord
        FROM (
            SELECT d."ComparedWithLastYearSameMonth" AS yoy,
                (d."Date" / 100)::int * 12 + (d."Date" % 100)::int AS ord
            FROM "Revenue" AS d
            WHERE d."stock_symbol" = r."stock_symbol" AND d."Date" <= r."Date"
            ORDER BY d."Date" DESC
            LIMIT 12
        ) AS recent
    ) AS ranked
) AS decline ON TRUE
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
            anchor_quarter_no: row.try_get("anchor_quarter_no")?,
            ttm_eps: row.try_get("ttm_eps")?,
            ttm_revenue: row.try_get("ttm_revenue")?,
            reg_slope: row.try_get("reg_slope")?,
            reg_intercept: row.try_get("reg_intercept")?,
            reg_r2: row.try_get("reg_r2")?,
            reg_quarters: row.try_get("reg_quarters")?,
            previous_high: row.try_get("previous_high")?,
            yoy_decline_months: row.try_get("yoy_decline_months")?,
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
    fs.operating_profit_margin,
    fs.net_income AS net_margin,
    prev.earnings_per_share AS last_year_earnings_per_share,
    prev.gross_profit AS last_year_gross_profit,
    prev.operating_profit_margin AS last_year_operating_margin,
    prev.net_income AS last_year_net_margin
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
            operating_margin: row.try_get("operating_profit_margin")?,
            net_margin: row.try_get("net_margin")?,
            last_year_earnings_per_share: row.try_get("last_year_earnings_per_share")?,
            last_year_gross_profit: row.try_get("last_year_gross_profit")?,
            last_year_operating_margin: row.try_get("last_year_operating_margin")?,
            last_year_net_margin: row.try_get("last_year_net_margin")?,
        })
    })
    .fetch_all(database::get_connection())
    .await?;

    Ok(alerts)
}

#[cfg(test)]
mod tests;
