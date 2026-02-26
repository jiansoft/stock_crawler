use anyhow::{anyhow, Result};
use chrono::NaiveDate;
use sqlx::postgres::PgQueryResult;

use crate::database;

/// 個股估值資料。
///
/// 彙整價格區間、股利法、EPS 法、PBR 法與 PER 法等估值結果，
/// 供排名與市場統計使用。
#[derive(sqlx::FromRow, Debug, Default)]
pub struct Estimate {
    /// 估值日期。
    pub date: NaiveDate,
    /// 參考的最後一筆日報價日期（字串格式）。
    pub last_daily_quote_date: String,
    /// 股票代號。
    pub security_code: String,
    /// 股票名稱。
    pub name: String,
    /// 當日收盤價。
    pub closing_price: f64,
    /// 估值百分比（收盤價相對便宜價）。
    pub percentage: f64,
    /// 加權便宜價。
    pub cheap: f64,
    /// 加權合理價。
    pub fair: f64,
    /// 加權昂貴價。
    pub expensive: f64,
    /// 價格法便宜價。
    pub price_cheap: f64,
    /// 價格法合理價。
    pub price_fair: f64,
    /// 價格法昂貴價。
    pub price_expensive: f64,
    /// 股利法便宜價。
    pub dividend_cheap: f64,
    /// 股利法合理價。
    pub dividend_fair: f64,
    /// 股利法昂貴價。
    pub dividend_expensive: f64,
    /// EPS 法便宜價。
    pub eps_cheap: f64,
    /// EPS 法合理價。
    pub eps_fair: f64,
    /// EPS 法昂貴價。
    pub eps_expensive: f64,
    /// PBR 法便宜價。
    pub pbr_cheap: f64,
    /// PBR 法合理價。
    pub pbr_fair: f64,
    /// PBR 法昂貴價。
    pub pbr_expensive: f64,
    /// 參與統計的年度數。
    pub year_count: i32,
    /// 內部排序或索引欄位。
    pub index: i32,
}

impl Estimate {
    /// 建立單一股票指定日期的估值模型預設值。
    pub fn new(security_code: String, date: NaiveDate) -> Self {
        Estimate {
            date,
            last_daily_quote_date: "".to_string(),
            security_code,
            name: "".to_string(),
            closing_price: 0.0,
            percentage: 0.0,
            cheap: 0.0,
            fair: 0.0,
            expensive: 0.0,
            price_cheap: 0.0,
            price_fair: 0.0,
            price_expensive: 0.0,
            dividend_cheap: 0.0,
            dividend_fair: 0.0,
            dividend_expensive: 0.0,
            eps_cheap: 0.0,
            eps_fair: 0.0,
            eps_expensive: 0.0,
            pbr_cheap: 0.0,
            pbr_fair: 0.0,
            pbr_expensive: 0.0,
            year_count: 0,
            index: 0,
        }
    }

    /// 依指定日期與年份清單，批次重建所有股票估值資料。
    ///
    /// `years` 格式為逗號分隔字串，例如 `\"2026,2025,2024\"`。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert_all(date: NaiveDate, years: String) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO estimate (
    security_code, "date", percentage, closing_price, cheap, fair, expensive, price_cheap,
    price_fair, price_expensive, dividend_cheap, dividend_fair, dividend_expensive, year_count,
    eps_cheap, eps_fair, eps_expensive, pbr_cheap, pbr_fair, pbr_expensive,
    per_cheap, per_fair, per_expensive, update_time
)
WITH filtered_years AS (
    -- 將字串年份轉為數組，支援參數化綁定，防範 SQL 注入
    SELECT CAST(string_to_array($2, ',') AS int[]) as years
),
daily_stats AS (
    -- 一次性計算所有基於 DailyQuotes 的統計指標，大幅減少 I/O
    SELECT
        dq."stock_symbol",
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."LowestPrice") AS p_cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."ClosingPrice") AS p_fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."HighestPrice") AS p_expensive,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") AS pbr_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") AS pbr_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") AS pbr_high,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") AS pe_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") AS pe_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") AS pe_high
    FROM "DailyQuotes" dq, filtered_years fy
    WHERE dq."Date" <= $1 
      AND dq."year" = ANY(fy.years)
      AND dq."ClosingPrice" > 0
    GROUP BY dq."stock_symbol"
),
dividend_agg AS (
    -- 股利聚合
    SELECT 
        security_code as stock_symbol,
        AVG(annual_sum) as div_base
    FROM (
        SELECT security_code, "year", SUM("sum") as annual_sum
        FROM dividend, filtered_years fy
        WHERE "year" = ANY(fy.years)
          AND ("ex-dividend_date1" != '-' OR "ex-dividend_date2" != '-')
        GROUP BY security_code, "year"
    ) t
    GROUP BY security_code
),
eps_per_agg AS (
    -- EPS 與財報統計
    SELECT 
        security_code as stock_symbol,
        AVG(annual_eps) as eps_avg
    FROM (
        SELECT security_code, "year", SUM(earnings_per_share) as annual_eps
        FROM financial_statement, filtered_years fy
        WHERE "year" = ANY(fy.years) AND quarter IN ('Q1','Q2','Q3','Q4')
        GROUP BY security_code, "year"
    ) t
    GROUP BY security_code
),
valuation_base AS (
    -- 統合所有估值方法所需的基礎指標
    SELECT
        s.stock_symbol,
        dq."Date" as q_date,
        dq."ClosingPrice" as q_close,
        ds.p_cheap, ds.p_fair, ds.p_expensive,
        (da.div_base * 15) as div_c, (da.div_base * 20) as div_f, (da.div_base * 25) as div_e,
        (s.last_four_eps * COALESCE(dpr.payout_ratio, 70) / 100 * 15) as eps_c,
        (s.last_four_eps * COALESCE(dpr.payout_ratio, 70) / 100 * 20) as eps_f,
        (s.last_four_eps * COALESCE(dpr.payout_ratio, 70) / 100 * 25) as eps_e,
        (ds.pbr_low * s.net_asset_value_per_share) as pbr_c,
        (ds.pbr_mid * s.net_asset_value_per_share) as pbr_f,
        (ds.pbr_high * s.net_asset_value_per_share) as pbr_e,
        (ds.pe_low * ep.eps_avg) as per_c,
        (ds.pe_mid * ep.eps_avg) as per_f,
        (ds.pe_high * ep.eps_avg) as per_e
    FROM stocks s
    JOIN "DailyQuotes" dq ON s.stock_symbol = dq."stock_symbol" AND dq."Date" = $1
    LEFT JOIN daily_stats ds ON s.stock_symbol = ds."stock_symbol"
    LEFT JOIN dividend_agg da ON s.stock_symbol = da.stock_symbol
    LEFT JOIN eps_per_agg ep ON s.stock_symbol = ep.stock_symbol
    LEFT JOIN (
        SELECT security_code, PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY payout_ratio) as payout_ratio
        FROM dividend, filtered_years fy WHERE "year" = ANY(fy.years) AND payout_ratio > 0 AND payout_ratio <= 200
        GROUP BY security_code
    ) dpr ON s.stock_symbol = dpr.security_code
    WHERE s."SuspendListing" = FALSE
)
SELECT
    stock_symbol, q_date,
    -- 使用加權後的便宜價作為分母計算百分比
    (q_close / NULLIF(calc.weighted_cheap, 0)) * 100,
    q_close, calc.weighted_cheap, calc.weighted_fair, calc.weighted_expensive,
    p_cheap, p_fair, p_expensive,
    div_c, div_f, div_e,
    0 as year_count,
    eps_c, eps_f, eps_e,
    pbr_c, pbr_f, pbr_e,
    per_c, per_f, per_e,
    NOW()
FROM valuation_base vb
CROSS JOIN LATERAL (
    -- 集中計算加權估值，提升性能與代碼可維護性
    SELECT 
        (COALESCE(p_cheap,0)*0.2 + COALESCE(div_c,0)*0.29 + COALESCE(eps_c,0)*0.3 + COALESCE(pbr_c,0)*0.2 + COALESCE(per_c,0)*0.01) as weighted_cheap,
        (COALESCE(p_fair,0)*0.2 + COALESCE(div_f,0)*0.29 + COALESCE(eps_f,0)*0.3 + COALESCE(pbr_f,0)*0.2 + COALESCE(per_f,0)*0.01) as weighted_fair,
        (COALESCE(p_expensive,0)*0.2 + COALESCE(div_e,0)*0.29 + COALESCE(eps_e,0)*0.3 + COALESCE(pbr_e,0)*0.2 + COALESCE(per_e,0)*0.01) as weighted_expensive
) calc
ON CONFLICT (date, security_code) DO UPDATE SET
    percentage = EXCLUDED.percentage,
    closing_price = EXCLUDED.closing_price,
    cheap = EXCLUDED.cheap,
    fair = EXCLUDED.fair,
    expensive = EXCLUDED.expensive,
    price_cheap = EXCLUDED.price_cheap,
    price_fair = EXCLUDED.price_fair,
    price_expensive = EXCLUDED.price_expensive,
    dividend_cheap = EXCLUDED.dividend_cheap,
    dividend_fair = EXCLUDED.dividend_fair,
    dividend_expensive = EXCLUDED.dividend_expensive,
    eps_cheap = EXCLUDED.eps_cheap,
    eps_fair = EXCLUDED.eps_fair,
    eps_expensive = EXCLUDED.eps_expensive,
    year_count = EXCLUDED.year_count,
    pbr_cheap = EXCLUDED.pbr_cheap,
    pbr_fair = EXCLUDED.pbr_fair,
    pbr_expensive = EXCLUDED.pbr_expensive,
    per_cheap = EXCLUDED.per_cheap,
    per_fair = EXCLUDED.per_fair,
    per_expensive = EXCLUDED.per_expensive,
    update_time = NOW();
"#;
        sqlx::query(sql)
            .bind(date)
            .bind(&years)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert_all() from database for date: {} with years: {}. Error: {:?}",
                    date,
                    years,
                    why,
                )
            })
    }


    /// 只重算單一股票的估值資料。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert(&self, years: String) -> Result<PgQueryResult> {
        let sql = r#"
INSERT INTO estimate (
    security_code, "date", percentage, closing_price, cheap, fair, expensive,
    price_cheap, price_fair, price_expensive,
    dividend_cheap, dividend_fair, dividend_expensive,
    eps_cheap, eps_fair, eps_expensive,
    pbr_cheap, pbr_fair, pbr_expensive,
    per_cheap, per_fair, per_expensive,
    year_count, update_time
)
WITH filtered_years AS (
    -- 參數化年份過濾
    SELECT CAST(string_to_array($2, ',') AS int[]) as years
),
daily_stats AS (
    -- 統合單一股票的所有百分位數統計
    SELECT
        dq."stock_symbol",
        COUNT(DISTINCT dq."year") AS y_count,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."LowestPrice") AS p_cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."ClosingPrice") AS p_fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."HighestPrice") AS p_expensive,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") AS pbr_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") AS pbr_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") AS pbr_high,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") AS pe_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") AS pe_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") AS pe_high
    FROM "DailyQuotes" dq, filtered_years fy
    WHERE dq."stock_symbol" = $3
      AND dq."Date" <= $1
      AND dq."year" = ANY(fy.years)
      AND dq."ClosingPrice" > 0
    GROUP BY dq."stock_symbol"
),
dividend_agg AS (
    SELECT 
        security_code,
        AVG(annual_sum) as div_base
    FROM (
        SELECT security_code, "year", SUM("sum") as annual_sum
        FROM dividend, filtered_years fy
        WHERE security_code = $3 AND "year" = ANY(fy.years)
        GROUP BY security_code, "year"
    ) t GROUP BY security_code
),
eps_agg AS (
    SELECT 
        security_code,
        AVG(annual_eps) as eps_avg
    FROM (
        SELECT security_code, "year", SUM(earnings_per_share) as annual_eps
        FROM financial_statement, filtered_years fy
        WHERE security_code = $3 AND "year" = ANY(fy.years) AND quarter IN ('Q1','Q2','Q3','Q4')
        GROUP BY security_code, "year"
    ) t GROUP BY security_code
),
valuation_base AS (
    SELECT
        s.stock_symbol,
        dq."Date" as q_date,
        dq."ClosingPrice" as q_close,
        ds.y_count, ds.p_cheap, ds.p_fair, ds.p_expensive,
        (da.div_base * 15) as div_c, (da.div_base * 20) as div_f, (da.div_base * 25) as div_e,
        (s.last_four_eps * COALESCE(dpr.payout_ratio, 70) / 100 * 15) as eps_c,
        (s.last_four_eps * COALESCE(dpr.payout_ratio, 70) / 100 * 20) as eps_f,
        (s.last_four_eps * COALESCE(dpr.payout_ratio, 70) / 100 * 25) as eps_e,
        (ds.pbr_low * s.net_asset_value_per_share) as pbr_c,
        (ds.pbr_mid * s.net_asset_value_per_share) as pbr_f,
        (ds.pbr_high * s.net_asset_value_per_share) as pbr_e,
        (ds.pe_low * ea.eps_avg) as per_c,
        (ds.pe_mid * ea.eps_avg) as per_f,
        (ds.pe_high * ea.eps_avg) as per_e
    FROM stocks s
    JOIN "DailyQuotes" dq ON s.stock_symbol = dq."stock_symbol" AND dq."Date" = $1
    LEFT JOIN daily_stats ds ON s.stock_symbol = ds."stock_symbol"
    LEFT JOIN dividend_agg da ON s.stock_symbol = da.security_code
    LEFT JOIN eps_agg ea ON s.stock_symbol = ea.security_code
    LEFT JOIN (
        SELECT security_code, COALESCE(PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY payout_ratio), 70) as payout_ratio
        FROM dividend, filtered_years fy WHERE security_code = $3 AND "year" = ANY(fy.years) AND payout_ratio > 0 AND payout_ratio <= 200
        GROUP BY security_code
    ) dpr ON s.stock_symbol = dpr.security_code
    WHERE s.stock_symbol = $3
)
SELECT
    stock_symbol, q_date,
    (q_close / NULLIF(calc.weighted_cheap, 0)) * 100,
    q_close, calc.weighted_cheap, calc.weighted_fair, calc.weighted_expensive,
    p_cheap, p_fair, p_expensive,
    div_c, div_f, div_e,
    eps_c, eps_f, eps_e,
    pbr_c, pbr_f, pbr_e,
    per_c, per_f, per_e,
    y_count, NOW()
FROM valuation_base vb
CROSS JOIN LATERAL (
    SELECT 
        (COALESCE(p_cheap,0)*0.2 + COALESCE(div_c,0)*0.29 + COALESCE(eps_c,0)*0.3 + COALESCE(pbr_c,0)*0.2 + COALESCE(per_c,0)*0.01) as weighted_cheap,
        (COALESCE(p_fair,0)*0.2 + COALESCE(div_f,0)*0.29 + COALESCE(eps_f,0)*0.3 + COALESCE(pbr_f,0)*0.2 + COALESCE(per_f,0)*0.01) as weighted_fair,
        (COALESCE(p_expensive,0)*0.2 + COALESCE(div_e,0)*0.29 + COALESCE(eps_e,0)*0.3 + COALESCE(pbr_e,0)*0.2 + COALESCE(per_e,0)*0.01) as weighted_expensive
) calc
ON CONFLICT (date, security_code) DO UPDATE SET
    percentage = EXCLUDED.percentage,
    closing_price = EXCLUDED.closing_price,
    cheap = EXCLUDED.cheap,
    fair = EXCLUDED.fair,
    expensive = EXCLUDED.expensive,
    update_time = NOW();
"#;

        sqlx::query(sql)
            .bind(self.date)
            .bind(&years)
            .bind(&self.security_code)
            .execute(database::get_connection())
            .await
            .map_err(|why| {
                anyhow!(
                    "Failed to upsert({:#?}) from database for years: {}. Error: {:?}",
                    self,
                    years,
                    why,
                )
            })
    }

}

#[cfg(test)]
mod tests {
    use crate::{cache::SHARE, logging};
    use chrono::Datelike;

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn test_upsert() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 Estimate::upsert".to_string());

        let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        let estimate = Estimate::new("9921".to_string(), current_date);

        match estimate.upsert(years_str).await {
            Ok(r) => logging::debug_file_async(format!("Estimate::upsert:{:#?}", r)),
            Err(why) => {
                logging::debug_file_async(format!("Failed to Estimate::upsert because {:?}", why));
            }
        }

        logging::debug_file_async("結束 Estimate::upsert".to_string());
    }

    #[tokio::test]
    #[ignore]
    async fn test_upsert_all() {
        dotenv::dotenv().ok();
        SHARE.load().await;
        logging::debug_file_async("開始 Estimate::upsert_all".to_string());

        let current_date = NaiveDate::parse_from_str("2023-10-20", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        match Estimate::upsert_all(current_date, years_str).await {
            Ok(r) => logging::debug_file_async(format!("Estimate::upsert_all:{:#?}", r)),
            Err(why) => {
                logging::debug_file_async(format!(
                    "Failed to Estimate::upsert_all because {:?}",
                    why
                ));
            }
        }

        logging::debug_file_async("結束 Estimate::upsert_all".to_string());
    }
}
