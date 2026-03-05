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
    /// ### 估值計算公式說明：
    /// 本方法整合五種估值模型，並依特定權重計算加權後的「便宜價」、「合理價」與「昂貴價」。
    ///
    /// **1. 加權比例 (Weights)：**
    /// *   價格法 (20%) + 股利法 (29%) + EPS 法 (30%) + PBR 法 (20%) + PER 法 (1%)
    ///
    /// **2. 各別估值法細節：**
    /// *   **價格法 (Price-based)：**
    ///     *   便宜價：指定年份內 `LowestPrice` 的 10% 分位數。
    ///     *   合理價：指定年份內 `ClosingPrice` 的 50% 分位數。
    ///     *   昂貴價：指定年份內 `HighestPrice` 的 80% 分位數。
    /// *   **股利法 (Dividend-based)：**
    ///     *   基準：指定年份內「年均股利」。
    ///     *   便宜/合理/昂貴：基準 × 15 / 20 / 25。
    /// *   **EPS 法 (Expected EPS)：**
    ///     *   基準：`近四季 EPS` × `指定年份內第 70 百分位的盈餘分配率 (Payout Ratio)`。
    ///     *   便宜/合理/昂貴：基準 × 15 / 20 / 25。
    /// *   **PBR 法 (Price-to-Book Ratio)：**
    ///     *   基準：`每股淨值`。
    ///     *   倍數：指定年份內 `PBR` 的 10% / 50% / 80% 分位數。
    ///     *   便宜/合理/昂貴：基準 × 倍數。
    /// *   **PER 法 (Price-Earning Ratio)：**
    ///     *   基準：指定年份內「年均 EPS」。
    ///     *   倍數：指定年份內 `PER` 的 10% / 50% / 80% 分位數。
    ///     *   便宜/合理/昂貴：基準 × 倍數。
    ///
    /// **3. 百分比 (Percentage) 計算：**
    /// *   公式：`(當前收盤價 / 加權便宜價) * 100`。
    /// *   數值越低代表股價相對越便宜。
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
stocks AS (
    -- 1. 基礎資料 CTE，包含產業 ID 以供 Fallback 使用
    SELECT 
        stock_symbol, last_four_eps, net_asset_value_per_share, stock_industry_id
    FROM public.stocks WHERE "SuspendListing" = false
),
daily_stats AS (
    -- 2. 一次性計算所有基於 DailyQuotes 的統計指標，大幅減少 I/O
    SELECT
        dq."stock_symbol",
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."LowestPrice")
            FILTER (WHERE dq."ClosingPrice" > 0) AS p_cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."ClosingPrice")
            FILTER (WHERE dq."ClosingPrice" > 0) AS p_fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."HighestPrice")
            FILTER (WHERE dq."ClosingPrice" > 0) AS p_expensive,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."price-to-book_ratio")
            FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."price-to-book_ratio")
            FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."price-to-book_ratio")
            FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_high,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."PriceEarningRatio")
            FILTER (WHERE dq."PriceEarningRatio" > 0) AS pe_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."PriceEarningRatio")
            FILTER (WHERE dq."PriceEarningRatio" > 0) AS pe_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."PriceEarningRatio")
            FILTER (WHERE dq."PriceEarningRatio" > 0) AS pe_high
    FROM "DailyQuotes" dq, filtered_years fy
    WHERE dq."Date" <= $1 
      AND dq."year" = ANY(fy.years)
    GROUP BY dq."stock_symbol"
),
-- 3. 年度股利彙總 (含有效性檢查)
annual_dividend AS (
    SELECT security_code, "year", SUM("sum") as annual_sum
    FROM dividend, filtered_years fy
    WHERE "year" = ANY(fy.years)
      AND ("ex-dividend_date1" != '-' OR "ex-dividend_date2" != '-')
    GROUP BY security_code, "year"
),
-- 4. 年度 EPS 彙總
annual_eps AS (
    SELECT security_code, "year", SUM(earnings_per_share) as annual_eps
    FROM financial_statement, filtered_years fy
    WHERE "year" = ANY(fy.years) AND quarter IN ('Q1','Q2','Q3','Q4')
    GROUP BY security_code, "year"
),
-- 5. 計算歷史分配率 (Payout Ratio) 並進行三層 Fallback 準備
payout_history AS (
    SELECT 
        ad.security_code, s.stock_industry_id,
        (ad.annual_sum::numeric / NULLIF(ae.annual_eps::numeric, 0)) * 100 as ratio
    FROM annual_dividend ad
    JOIN annual_eps ae ON ad.security_code = ae.security_code AND ad.year = ae.year
    JOIN stocks s ON ad.security_code = s.stock_symbol
    WHERE ae.annual_eps > 0 AND ad.annual_sum > 0 AND (ad.annual_sum / ae.annual_eps) <= 2.0
),
stock_payout_70th AS (
    SELECT security_code, PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY ratio) as stock_payout
    FROM payout_history GROUP BY security_code
),
industry_payout_70th AS (
    SELECT stock_industry_id, PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY ratio) as industry_payout
    FROM payout_history GROUP BY stock_industry_id
),
final_dpr AS (
    -- 三層 Fallback: 個股歷史 -> 產業歷史 -> 固定常數 70.0
    SELECT 
        s.stock_symbol,
        LEAST(
            GREATEST(COALESCE(sp.stock_payout, ip.industry_payout, 70.0), 0.0),
            200.0
        ) as payout_ratio
    FROM stocks s
    LEFT JOIN stock_payout_70th sp ON s.stock_symbol = sp.security_code
    LEFT JOIN industry_payout_70th ip ON s.stock_industry_id = ip.stock_industry_id
),
valuation_base AS (
    -- 6. 統合所有估值方法所需的基礎指標
    SELECT
        s.stock_symbol,
        dq."Date" as q_date,
        dq."ClosingPrice" as q_close,
        ds.p_cheap, ds.p_fair, ds.p_expensive,
        (COALESCE(ad_avg.avg_div, 0) * 15) as div_c,
        (COALESCE(ad_avg.avg_div, 0) * 20) as div_f,
        (COALESCE(ad_avg.avg_div, 0) * 25) as div_e,
        CASE
            WHEN s.last_four_eps > 0 THEN s.last_four_eps * (fd.payout_ratio / 100.0) * 15
            ELSE 0
        END as eps_c,
        CASE
            WHEN s.last_four_eps > 0 THEN s.last_four_eps * (fd.payout_ratio / 100.0) * 20
            ELSE 0
        END as eps_f,
        CASE
            WHEN s.last_four_eps > 0 THEN s.last_four_eps * (fd.payout_ratio / 100.0) * 25
            ELSE 0
        END as eps_e,
        (ds.pbr_low * s.net_asset_value_per_share) as pbr_c,
        (ds.pbr_mid * s.net_asset_value_per_share) as pbr_f,
        (ds.pbr_high * s.net_asset_value_per_share) as pbr_e,
        CASE
            WHEN ae_avg.avg_eps > 0 THEN ds.pe_low * ae_avg.avg_eps
            ELSE 0
        END as per_c,
        CASE
            WHEN ae_avg.avg_eps > 0 THEN ds.pe_mid * ae_avg.avg_eps
            ELSE 0
        END as per_f,
        CASE
            WHEN ae_avg.avg_eps > 0 THEN ds.pe_high * ae_avg.avg_eps
            ELSE 0
        END as per_e
    FROM stocks s
    JOIN "DailyQuotes" dq ON s.stock_symbol = dq."stock_symbol" AND dq."Date" = $1
    JOIN daily_stats ds ON s.stock_symbol = ds."stock_symbol"
        AND ds.p_cheap IS NOT NULL
        AND ds.pbr_low IS NOT NULL
        AND ds.pe_low IS NOT NULL
    JOIN final_dpr fd ON s.stock_symbol = fd.stock_symbol
    LEFT JOIN (SELECT security_code, AVG(annual_sum) as avg_div FROM annual_dividend GROUP BY security_code) ad_avg ON s.stock_symbol = ad_avg.security_code
    LEFT JOIN (SELECT security_code, AVG(annual_eps) as avg_eps FROM annual_eps GROUP BY security_code) ae_avg ON s.stock_symbol = ae_avg.security_code
)
SELECT
    stock_symbol, q_date,
    -- 使用加權後的便宜價作為分母計算百分比
    CASE
        WHEN calc.weighted_cheap > 0 THEN ROUND(((q_close / calc.weighted_cheap) * 100)::numeric, 4)
        ELSE NULL
    END as percentage,
    ROUND(q_close::numeric, 4) as q_close,
    ROUND(calc.weighted_cheap::numeric, 4) as weighted_cheap,
    ROUND(calc.weighted_fair::numeric, 4) as weighted_fair,
    ROUND(calc.weighted_expensive::numeric, 4) as weighted_expensive,
    ROUND(p_cheap::numeric, 4) as p_cheap,
    ROUND(p_fair::numeric, 4) as p_fair,
    ROUND(p_expensive::numeric, 4) as p_expensive,
    ROUND(div_c::numeric, 4) as div_c,
    ROUND(div_f::numeric, 4) as div_f,
    ROUND(div_e::numeric, 4) as div_e,
    0 as year_count,
    ROUND(eps_c::numeric, 4) as eps_c,
    ROUND(eps_f::numeric, 4) as eps_f,
    ROUND(eps_e::numeric, 4) as eps_e,
    ROUND(pbr_c::numeric, 4) as pbr_c,
    ROUND(pbr_f::numeric, 4) as pbr_f,
    ROUND(pbr_e::numeric, 4) as pbr_e,
    ROUND(per_c::numeric, 4) as per_c,
    ROUND(per_f::numeric, 4) as per_f,
    ROUND(per_e::numeric, 4) as per_e,
    NOW() as now
FROM valuation_base vb
CROSS JOIN LATERAL (
    -- 集中計算加權估值，提升性能與代碼可維護性
    SELECT 
        (COALESCE(p_cheap, 0)*0.2 + COALESCE(div_c, 0)*0.29 + COALESCE(eps_c, 0)*0.3 + COALESCE(pbr_c, 0)*0.2 + COALESCE(per_c, 0)*0.01) as weighted_cheap,
        (COALESCE(p_fair, 0)*0.2 + COALESCE(div_f, 0)*0.29 + COALESCE(eps_f, 0)*0.3 + COALESCE(pbr_f, 0)*0.2 + COALESCE(per_f, 0)*0.01) as weighted_fair,
        (COALESCE(p_expensive, 0)*0.2 + COALESCE(div_e, 0)*0.29 + COALESCE(eps_e, 0)*0.3 + COALESCE(pbr_e, 0)*0.2 + COALESCE(per_e, 0)*0.01) as weighted_expensive
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
stocks AS (
    -- 1. 基礎資料 CTE，包含產業 ID 以供 Fallback 使用
    SELECT 
        stock_symbol, last_four_eps, net_asset_value_per_share, stock_industry_id
    FROM public.stocks WHERE stock_symbol = $3 AND "SuspendListing" = false
),
daily_stats AS (
    -- 2. 統合單一股票的所有百分位數統計
    SELECT
        dq."stock_symbol",
        COUNT(DISTINCT dq."year")
            FILTER (WHERE dq."ClosingPrice" > 0) AS y_count,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."LowestPrice")
            FILTER (WHERE dq."ClosingPrice" > 0) AS p_cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."ClosingPrice")
            FILTER (WHERE dq."ClosingPrice" > 0) AS p_fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."HighestPrice")
            FILTER (WHERE dq."ClosingPrice" > 0) AS p_expensive,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."price-to-book_ratio")
            FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."price-to-book_ratio")
            FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."price-to-book_ratio")
            FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_high,
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."PriceEarningRatio")
            FILTER (WHERE dq."PriceEarningRatio" > 0) AS pe_low,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."PriceEarningRatio")
            FILTER (WHERE dq."PriceEarningRatio" > 0) AS pe_mid,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."PriceEarningRatio")
            FILTER (WHERE dq."PriceEarningRatio" > 0) AS pe_high
    FROM "DailyQuotes" dq, filtered_years fy
    WHERE dq."stock_symbol" = $3
      AND dq."Date" <= $1
      AND dq."year" = ANY(fy.years)
    GROUP BY dq."stock_symbol"
),
-- 3. 年度股利彙總 (含有效性檢查)
annual_dividend AS (
    SELECT security_code, "year", SUM("sum") as annual_sum
    FROM dividend, filtered_years fy
    WHERE security_code = $3 AND "year" = ANY(fy.years)
      AND ("ex-dividend_date1" != '-' OR "ex-dividend_date2" != '-')
    GROUP BY security_code, "year"
),
-- 4. 年度 EPS 彙總
annual_eps AS (
    SELECT security_code, "year", SUM(earnings_per_share) as annual_eps
    FROM financial_statement, filtered_years fy
    WHERE security_code = $3 AND "year" = ANY(fy.years) AND quarter IN ('Q1','Q2','Q3','Q4')
    GROUP BY security_code, "year"
),
-- 5. 計算分配率並進行 Fallback (單筆仍維持與批次相同的回退邏輯)
payout_history_all AS (
    SELECT 
        ad.security_code, s.stock_industry_id,
        (ad.annual_sum::numeric / NULLIF(ae.annual_eps::numeric, 0)) * 100 as ratio
    FROM annual_dividend ad
    JOIN annual_eps ae ON ad.security_code = ae.security_code AND ad.year = ae.year
    JOIN stocks s ON ad.security_code = s.stock_symbol
    WHERE ae.annual_eps > 0 AND ad.annual_sum > 0 AND (ad.annual_sum / ae.annual_eps) <= 2.0
),
stock_payout_70th AS (
    SELECT security_code, PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY ratio) as stock_payout
    FROM payout_history_all GROUP BY security_code
),
industry_annual_dividend AS (
    SELECT d.security_code, d."year", SUM(d."sum") as annual_sum, s.stock_industry_id
    FROM dividend d
    JOIN public.stocks s ON d.security_code = s.stock_symbol
    WHERE s.stock_industry_id = (SELECT stock_industry_id FROM stocks)
      AND d."year" = ANY((SELECT years FROM filtered_years))
      AND (d."ex-dividend_date1" != '-' OR d."ex-dividend_date2" != '-')
    GROUP BY d.security_code, d."year", s.stock_industry_id
),
industry_annual_eps AS (
    SELECT fs.security_code, fs."year", SUM(fs.earnings_per_share) as annual_eps
    FROM financial_statement fs
    JOIN public.stocks s ON fs.security_code = s.stock_symbol
    WHERE s.stock_industry_id = (SELECT stock_industry_id FROM stocks)
      AND fs."year" = ANY((SELECT years FROM filtered_years))
      AND fs.quarter IN ('Q1','Q2','Q3','Q4')
    GROUP BY fs.security_code, fs."year"
),
industry_payout_history AS (
    SELECT
        iad.stock_industry_id,
        (iad.annual_sum::numeric / NULLIF(iae.annual_eps::numeric, 0)) * 100 as ratio
    FROM industry_annual_dividend iad
    JOIN industry_annual_eps iae
      ON iad.security_code = iae.security_code
     AND iad."year" = iae."year"
    WHERE iae.annual_eps > 0
      AND iad.annual_sum > 0
      AND (iad.annual_sum::numeric / iae.annual_eps::numeric) <= 2.0
),
industry_payout_70th AS (
    SELECT stock_industry_id, PERCENTILE_CONT(0.7) WITHIN GROUP (ORDER BY ratio) as industry_payout
    FROM industry_payout_history
    GROUP BY stock_industry_id
),
final_dpr AS (
    SELECT 
        s.stock_symbol,
        LEAST(
            GREATEST(COALESCE(sp.stock_payout, ip.industry_payout, 70.0), 0.0),
            200.0
        ) as payout_ratio
    FROM stocks s
    LEFT JOIN stock_payout_70th sp ON s.stock_symbol = sp.security_code
    LEFT JOIN industry_payout_70th ip ON s.stock_industry_id = ip.stock_industry_id
),
valuation_base AS (
    SELECT
        s.stock_symbol, dq."Date" as q_date, dq."ClosingPrice" as q_close,
        ds.y_count, ds.p_cheap, ds.p_fair, ds.p_expensive,
        (COALESCE(ad_avg.avg_div, 0) * 15) as div_c, (COALESCE(ad_avg.avg_div, 0) * 20) as div_f, (COALESCE(ad_avg.avg_div, 0) * 25) as div_e,
        CASE
            WHEN s.last_four_eps > 0 THEN s.last_four_eps * (fd.payout_ratio / 100.0) * 15
            ELSE 0
        END as eps_c,
        CASE
            WHEN s.last_four_eps > 0 THEN s.last_four_eps * (fd.payout_ratio / 100.0) * 20
            ELSE 0
        END as eps_f,
        CASE
            WHEN s.last_four_eps > 0 THEN s.last_four_eps * (fd.payout_ratio / 100.0) * 25
            ELSE 0
        END as eps_e,
        (ds.pbr_low * s.net_asset_value_per_share) as pbr_c,
        (ds.pbr_mid * s.net_asset_value_per_share) as pbr_f,
        (ds.pbr_high * s.net_asset_value_per_share) as pbr_e,
        CASE
            WHEN ae_avg.avg_eps > 0 THEN ds.pe_low * ae_avg.avg_eps
            ELSE 0
        END as per_c,
        CASE
            WHEN ae_avg.avg_eps > 0 THEN ds.pe_mid * ae_avg.avg_eps
            ELSE 0
        END as per_f,
        CASE
            WHEN ae_avg.avg_eps > 0 THEN ds.pe_high * ae_avg.avg_eps
            ELSE 0
        END as per_e
    FROM stocks s
    JOIN "DailyQuotes" dq ON s.stock_symbol = dq."stock_symbol" AND dq."Date" = $1
    JOIN daily_stats ds ON s.stock_symbol = ds."stock_symbol"
    JOIN final_dpr fd ON s.stock_symbol = fd.stock_symbol
    LEFT JOIN (SELECT security_code, AVG(annual_sum) as avg_div FROM annual_dividend GROUP BY security_code) ad_avg ON s.stock_symbol = ad_avg.security_code
    LEFT JOIN (SELECT security_code, AVG(annual_eps) as avg_eps FROM annual_eps GROUP BY security_code) ae_avg ON s.stock_symbol = ae_avg.security_code
)
SELECT
    stock_symbol, q_date,
    CASE
        WHEN calc.weighted_cheap > 0 THEN ROUND(((q_close / calc.weighted_cheap) * 100)::numeric, 4)
        ELSE NULL
    END as percentage,
    ROUND(q_close::numeric, 4) as q_close,
    ROUND(calc.weighted_cheap::numeric, 4) as weighted_cheap,
    ROUND(calc.weighted_fair::numeric, 4) as weighted_fair,
    ROUND(calc.weighted_expensive::numeric, 4) as weighted_expensive,
    ROUND(p_cheap::numeric, 4) as p_cheap,
    ROUND(p_fair::numeric, 4) as p_fair,
    ROUND(p_expensive::numeric, 4) as p_expensive,
    ROUND(div_c::numeric, 4) as div_c,
    ROUND(div_f::numeric, 4) as div_f,
    ROUND(div_e::numeric, 4) as div_e,
    ROUND(eps_c::numeric, 4) as eps_c,
    ROUND(eps_f::numeric, 4) as eps_f,
    ROUND(eps_e::numeric, 4) as eps_e,
    ROUND(pbr_c::numeric, 4) as pbr_c,
    ROUND(pbr_f::numeric, 4) as pbr_f,
    ROUND(pbr_e::numeric, 4) as pbr_e,
    ROUND(per_c::numeric, 4) as per_c,
    ROUND(per_f::numeric, 4) as per_f,
    ROUND(per_e::numeric, 4) as per_e,
    y_count, NOW() as now
FROM valuation_base vb
CROSS JOIN LATERAL (
    SELECT 
        (COALESCE(p_cheap, 0)*0.2 + COALESCE(div_c, 0)*0.29 + COALESCE(eps_c, 0)*0.3 + COALESCE(pbr_c, 0)*0.2 + COALESCE(per_c, 0)*0.01) as weighted_cheap,
        (COALESCE(p_fair, 0)*0.2 + COALESCE(div_f, 0)*0.29 + COALESCE(eps_f, 0)*0.3 + COALESCE(pbr_f, 0)*0.2 + COALESCE(per_f, 0)*0.01) as weighted_fair,
        (COALESCE(p_expensive, 0)*0.2 + COALESCE(div_e, 0)*0.29 + COALESCE(eps_e, 0)*0.3 + COALESCE(pbr_e, 0)*0.2 + COALESCE(per_e, 0)*0.01) as weighted_expensive
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
    pbr_cheap = EXCLUDED.pbr_cheap,
    pbr_fair = EXCLUDED.pbr_fair,
    pbr_expensive = EXCLUDED.pbr_expensive,
    per_cheap = EXCLUDED.per_cheap,
    per_fair = EXCLUDED.per_fair,
    per_expensive = EXCLUDED.per_expensive,
    year_count = EXCLUDED.year_count,
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

        let current_date = NaiveDate::parse_from_str("2026-03-03", "%Y-%m-%d").unwrap();
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
