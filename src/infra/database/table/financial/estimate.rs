use anyhow::{Result, anyhow};
use chrono::NaiveDate;
use sqlx::postgres::PgQueryResult;

use crate::{core::declare::Industry, infra::database};

/// 個股估值資料。
///
/// 彙整價格法、股利法、PBR 法、PER 法的加權估值，以及僅供參考的預估股利，
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
    /// 預估股利法便宜價（不計入加權）。
    pub eps_cheap: f64,
    /// 預估股利法合理價（不計入加權）。
    pub eps_fair: f64,
    /// 預估股利法昂貴價（不計入加權）。
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
    /// 以價格法、股利法、PBR 法、PER 法四種模型加權出「便宜價」、「合理價」與「昂貴價」；
    /// 另存一組預估股利（`eps_*` 欄位）供參考，不計入加權。
    ///
    /// **1. 加權比例 (Weights)：**
    /// *   價格法 (20%) + 股利法 (25%) + PBR 法 (20%) + PER 法 (35%)
    /// *   各法便宜/合理/昂貴三價都 > 0 才算有效；**缺項不以 0 計入**，權重在有效方法之間按比例重新分配。
    /// *   PER 法計入加權時，不超過「所有有效方法（含 PER）同一價位中位數」的 3 倍（`per_*` 欄位仍存原值）：
    ///     景氣循環股在獲利高峰時本益比位階 × 高峰 EPS 會嚴重高估（青雲 PER 合理 1,895、股價 290）。
    ///     中位數含 PER 本身：只排除其他三法會被看歷史的價格法、股利法拉低，連台積電都被誤截。
    /// *   有效方法少於 2 個時不給估價（加權三價與百分比都寫 0；percentage 欄位不允許 NULL）。
    ///
    /// **2. 各別估值法細節：**
    /// *   **價格法 (Price-based)：**
    ///     *   便宜價：指定年份內 `LowestPrice` 的 10% 分位數。
    ///     *   合理價：指定年份內 `ClosingPrice` 的 50% 分位數。
    ///     *   昂貴價：指定年份內 `HighestPrice` 的 80% 分位數。
    /// *   **股利法 (Dividend-based)：**
    ///     *   基準：指定年份內「年均股利」。
    ///     *   便宜/合理/昂貴：基準 × 15 / 20 / 25。
    /// *   **預估股利（`eps_*` 欄位，不計入加權）：**
    ///     *   基準：`近四季 EPS` × `各發放年度盈餘分配率的中位數`（個股 → 產業 → 60%）。
    ///     *   盈餘分配率取 `dividend.payout_ratio`（股利 ÷ 涵蓋期間 EPS），不再用同一年的 EPS 相除。
    ///     *   便宜/合理/昂貴：基準 × 15 / 20 / 25。
    ///     *   本質與股利法同為殖利率法；計入加權會讓殖利率重複計權，低估低配發率的成長股。
    /// *   **PBR 法 (Price-to-Book Ratio)：**
    ///     *   基準：最新一季 `每股淨值`。
    ///     *   倍數：指定年份內 `PBR` 的 10% / 50% / 80% 分位數。
    ///     *   便宜/合理/昂貴：基準 × 倍數。
    /// *   **PER 法 (Price-Earning Ratio，本益比河流圖)：**
    ///     *   基準：`近四季 EPS`（與資料庫本益比同一口徑）。
    ///     *   倍數：指定年份內 `PER` 的 10% / 50% / 80% 分位數，只取 0～100 倍；
    ///         有效天數不足 250 個交易日時 PER 法無效。
    ///
    /// **3. 分割／減資還原（`corporate_action`）：**
    /// *   報價、股利、財報都是事件當時的每股數字；事件之後股數改變，舊數字要除以「之後所有事件
    ///     股數比例的乘積」才能和現在的股價比較（5904 寶雅 1:10 分割後收盤 69.7，未還原的合理價 568）。
    /// *   歷史股價依報價日、股利依除權息日、近四季 EPS 與每股淨值依各季季底換算。
    /// *   本益比、PBR 是比率不受影響；配發率是原始股利 ÷ 涵蓋期間原始 EPS，比例同樣不受影響。
    ///
    /// **4. 百分比 (Percentage) 計算：**
    /// *   公式：`(當前收盤價 / 加權便宜價) * 100`。
    /// *   數值越低代表股價相對越便宜。
    ///
    /// `years` 格式為逗號分隔字串，例如 `\"2026,2025,2024\"`。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert_all(date: NaiveDate, years: String) -> Result<PgQueryResult> {
        // 依股票代號雜湊分成 ESTIMATE_BUCKETS 桶同時跑：瓶頸是 297 萬筆日K 的 5 組
        // 百分位數排序，ordered-set aggregate 無法平行化，單一語句只用得到一顆 CPU
        // （2026-10-08 正式庫實測 7.6～8.2 秒 → 4 桶 2.8～3.0 秒，結果逐筆相同）。
        // 各桶寫入的股票互不重疊，不會互相鎖住；任一桶失敗整體回報失敗，
        // 已完成的桶照常保留（與單檔重算相同的 upsert 語意，沒有先刪後寫）。
        let buckets = (0..ESTIMATE_BUCKETS)
            .map(|bucket| run_estimate_upsert(date, &years, None, (ESTIMATE_BUCKETS, bucket)));
        futures::future::try_join_all(buckets)
            .await
            .map(|results| {
                let mut total = PgQueryResult::default();
                total.extend(results);
                total
            })
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
    /// 與 [`Self::upsert_all`] 共用同一段 SQL，只多一個代號篩選：產業配發率中位數等
    /// 需要全市場資料的中間結果照常以全市場計算，單檔重算的結果才會和批次重建一致。
    ///
    /// # Errors
    /// 當 SQL 執行失敗時回傳錯誤。
    pub async fn upsert(&self, years: String) -> Result<PgQueryResult> {
        run_estimate_upsert(self.date, &years, Some(&self.security_code), (1, 0))
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

/// 全市場重建時同時執行的桶數（各佔一條連線，連線池上限 20）。
const ESTIMATE_BUCKETS: i32 = 4;

/// 執行估值重建 SQL；`security_code` 為 `None` 時重建全市場。
///
/// `(bucket_count, bucket)` 只處理代號雜湊落在第 `bucket` 桶的股票；`(1, 0)` 為不分桶。
async fn run_estimate_upsert(
    date: NaiveDate,
    years: &str,
    security_code: Option<&str>,
    (bucket_count, bucket): (i32, i32),
) -> std::result::Result<PgQueryResult, sqlx::Error> {
    sqlx::query(ESTIMATE_UPSERT_SQL)
        .bind(date)
        .bind(years)
        .bind(Industry::ExchangeTradedFund.serial())
        .bind(security_code)
        .bind(bucket_count)
        .bind(bucket)
        .execute(database::get_connection())
        .await
}

/// 估值重建 SQL，公式說明見 [`Estimate::upsert_all`]。
///
/// 參數：`$1` 估值日期、`$2` 逗號分隔年份、`$3` 排除的 ETF 產業代碼、`$4` 單一股票代號（NULL 為全市場）、
/// `$5` 桶數、`$6` 桶號（`$5 = 1` 為不分桶）。
const ESTIMATE_UPSERT_SQL: &str = r#"
INSERT INTO estimate (
    security_code, "date", percentage, closing_price, cheap, fair, expensive, price_cheap,
    price_fair, price_expensive, dividend_cheap, dividend_fair, dividend_expensive, year_count,
    eps_cheap, eps_fair, eps_expensive, pbr_cheap, pbr_fair, pbr_expensive,
    per_cheap, per_fair, per_expensive, update_time
)
WITH filtered_years AS (
    -- 將傳入的逗號分隔年份字串轉為整數陣列，供後續所有 CTE 過濾使用，避免多次解析字串。
    -- from_date 是最早年份的 1 月 1 日：daily_stats 以日期區間走 (stock_symbol, Date) 索引，
    -- 只用 year = ANY 時會掃完全部 660 萬筆日K 再過濾（2026-10 實測掃描 2.4 秒 → 1.4 秒）。
    SELECT p.years, make_date(m.min_year, 1, 1) AS from_date
    FROM (SELECT CAST(string_to_array($2, ',') AS int[]) AS years) p
    CROSS JOIN LATERAL (SELECT MIN(y) AS min_year FROM unnest(p.years) y) m
),
stocks AS (
    -- 篩選出目前未停止上市的股票，獲取其代號、最新的近四季 EPS、每股淨值與所屬產業 ID
    SELECT stock_symbol, last_four_eps, net_asset_value_per_share, stock_industry_id
    FROM public.stocks WHERE "SuspendListing" = false AND stock_industry_id != $3
),
action_ranges AS (
    -- 分割／減資還原係數：生效日之前的每股數字要除以「該日之後所有事件股數比例的乘積」。
    -- 相鄰兩事件之間 [from_date, to_date) 共用同一係數；沒有事件的股票不在此表（係數視為 1）。
    SELECT stock_symbol,
        LAG(effective_date) OVER (PARTITION BY stock_symbol ORDER BY effective_date) AS from_date,
        effective_date AS to_date,
        EXP(SUM(LN(share_ratio)) OVER (
            PARTITION BY stock_symbol ORDER BY effective_date DESC
            ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW
        )) AS factor
    FROM corporate_action
    WHERE effective_date <= $1 AND share_ratio > 0
),
daily_stats_raw AS (
    -- 核心統計 CTE：計算指定年份區間內，每支股票的價格位階與估值倍數位階
    SELECT
        dq."stock_symbol",
        -- 統計具有效成交價的年度總數，用來衡量估值樣本是否充足
        COUNT(DISTINCT dq."year") FILTER (WHERE dq."ClosingPrice" > 0) AS y_count,
        -- 價格法：股價先依分割／減資還原成現在的股數基準，再取最低價 10%、收盤 50%、最高價 80% 分位數
        PERCENTILE_CONT(0.1) WITHIN GROUP (ORDER BY dq."LowestPrice" / COALESCE(ar.factor, 1)) FILTER (WHERE dq."ClosingPrice" > 0) AS p_cheap,
        PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY dq."ClosingPrice" / COALESCE(ar.factor, 1)) FILTER (WHERE dq."ClosingPrice" > 0) AS p_fair,
        PERCENTILE_CONT(0.8) WITHIN GROUP (ORDER BY dq."HighestPrice" / COALESCE(ar.factor, 1)) FILTER (WHERE dq."ClosingPrice" > 0) AS p_expensive,
        -- PBR 法：取歷史股價淨值比的 10% / 50% / 80% 位階（比率，不受分割影響）。
        -- 同一欄的三個位階用陣列版 PERCENTILE_CONT 一次算完，每組只排序一次（結果與分開算相同）。
        PERCENTILE_CONT(ARRAY[0.1, 0.5, 0.8]) WITHIN GROUP (ORDER BY dq."price-to-book_ratio") FILTER (WHERE dq."price-to-book_ratio" > 0) AS pbr_pct,
        -- PER 法：取歷史本益比的 10% / 50% / 80% 位階。
        -- 本益比 > 100 代表當時獲利趨近於 0，不是有意義的評價倍數（台船 6 年中位數 2,050 倍），一律排除；
        -- pe_days 記錄有效天數，不足一年（250 個交易日）時 PER 法視為無效。
        PERCENTILE_CONT(ARRAY[0.1, 0.5, 0.8]) WITHIN GROUP (ORDER BY dq."PriceEarningRatio") FILTER (WHERE dq."PriceEarningRatio" > 0 AND dq."PriceEarningRatio" <= 100) AS pe_pct,
        COUNT(*) FILTER (WHERE dq."PriceEarningRatio" > 0 AND dq."PriceEarningRatio" <= 100) AS pe_days
    -- 只統計 stocks CTE 內的股票：ETF、下市股的日K 最後也不會輸出，先排除少排序約 8%。
    FROM "DailyQuotes" dq
    JOIN stocks s ON s.stock_symbol = dq.stock_symbol
    CROSS JOIN filtered_years fy
    LEFT JOIN action_ranges ar ON ar.stock_symbol = dq.stock_symbol
        AND dq."Date" < ar.to_date AND (ar.from_date IS NULL OR dq."Date" >= ar.from_date)
    WHERE dq."Date" >= fy.from_date AND dq."Date" <= $1 AND dq."year" = ANY(fy.years)
      AND ($4::varchar IS NULL OR dq.stock_symbol = $4)
      -- 分桶：後續 CTE 都以 daily_stats 為主表，這裡篩掉的股票不會輸出；
      -- 轉 bigint 再取絕對值，避免 hashtext 回傳 int 最小值時 abs() 溢位。
      AND ($5::int = 1 OR abs(hashtext(dq.stock_symbol)::bigint) % $5::int = $6::int)
    GROUP BY dq."stock_symbol"
),
daily_stats AS (
    -- 把 PBR／PER 的位階陣列展開成個別欄位，後續 CTE 沿用原欄位名稱
    SELECT stock_symbol, y_count, p_cheap, p_fair, p_expensive,
        pbr_pct[1] AS pbr_low, pbr_pct[2] AS pbr_mid, pbr_pct[3] AS pbr_high,
        pe_pct[1] AS pe_low, pe_pct[2] AS pe_mid, pe_pct[3] AS pe_high, pe_days
    FROM daily_stats_raw
),
adjusted_annual_dividend AS (
    -- 股利法用的年度配息：每筆股利依除權息日之後的分割／減資還原成現在的股數基準
    SELECT d.security_code, d."year", SUM(d."sum" / COALESCE(ar.factor, 1)) AS annual_sum
    FROM dividend d
    CROSS JOIN filtered_years fy
    CROSS JOIN LATERAL (
        SELECT CASE
            WHEN d."ex-dividend_date1" ~ '^\d{4}-\d{2}-\d{2}$' THEN d."ex-dividend_date1"::date
            WHEN d."ex-dividend_date2" ~ '^\d{4}-\d{2}-\d{2}$' THEN d."ex-dividend_date2"::date
        END AS ex_date
    ) x
    LEFT JOIN action_ranges ar ON ar.stock_symbol = d.security_code
        AND x.ex_date < ar.to_date AND (ar.from_date IS NULL OR x.ex_date >= ar.from_date)
    WHERE d."year" = ANY(fy.years) AND (d."ex-dividend_date1" != '-' OR d."ex-dividend_date2" != '-')
    GROUP BY d.security_code, d."year"
),
payout_history AS (
    -- 每個發放年度的盈餘分配率，直接採用 dividend.payout_ratio（股利 ÷ 涵蓋期間 EPS，02:30 排程計算）。
    -- 舊版用「Y 年發放的股利 ÷ Y 年 EPS」，年配股票的股利其實來自 Y-1 年盈餘，錯位一年
    -- （長榮 18.5% → 50.3%）。每個發放年度只取一列年度層級的紀錄（quarter = ''：單次年配本身，
    -- 或多次配發的年度合計列）；涵蓋期間 EPS ≤ 0 或配發率超過 200%（處分資產等異常配息）不列入。
    SELECT d.security_code, s.stock_industry_id, d.payout_ratio AS ratio
    FROM dividend d
    CROSS JOIN filtered_years fy
    JOIN stocks s ON d.security_code = s.stock_symbol
    WHERE d."year" = ANY(fy.years) AND d.quarter = ''
      AND d.payout_eps > 0 AND d.payout_ratio > 0 AND d.payout_ratio <= 200
),
stock_payout_50th AS (
    -- 計算個股歷史配發率的中位數，作為預估股利的配發率基準
    SELECT security_code, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ratio) as stock_payout
    FROM payout_history GROUP BY security_code
),
industry_payout_50th AS (
    -- 計算同產業配發率的中位數，用於該股票缺乏歷史數據時的第二層 Fallback
    SELECT stock_industry_id, PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY ratio) as industry_payout
    FROM payout_history GROUP BY stock_industry_id
),
final_dpr AS (
    -- 決定最終採用的盈餘分配率：優先順序為「個股中位數 > 產業中位數 > 固定常數 60.0%」
    SELECT s.stock_symbol, LEAST(GREATEST(COALESCE(sp.stock_payout, ip.industry_payout, 60.0), 0.0), 200.0) as payout_ratio
    FROM stocks s
    LEFT JOIN stock_payout_50th sp ON s.stock_symbol = sp.security_code
    LEFT JOIN industry_payout_50th ip ON s.stock_industry_id = ip.stock_industry_id
),
latest_quarters AS (
    -- 今年與去年的季報（取數範圍同 stocks.last_four_eps），依新到舊編號並附上季底日期
    SELECT fs.security_code, fs.earnings_per_share, fs.net_asset_value_per_share,
        ROW_NUMBER() OVER (PARTITION BY fs.security_code ORDER BY fs.year DESC, fs.quarter DESC) AS rn,
        (make_date(fs.year::int, CASE fs.quarter WHEN 'Q1' THEN 3 WHEN 'Q2' THEN 6 WHEN 'Q3' THEN 9 ELSE 12 END, 1)
            + interval '1 month' - interval '1 day')::date AS period_end
    FROM financial_statement fs
    WHERE fs.quarter IN ('Q1','Q2','Q3','Q4')
      AND fs.year IN (EXTRACT(YEAR FROM $1::date)::int, EXTRACT(YEAR FROM $1::date)::int - 1)
),
per_share_basis AS (
    -- 近四季 EPS 與最新一季每股淨值，逐季依季底之後的分割／減資還原：
    -- 分割後的第一份季報已是新股數，其餘三季仍是舊股數，只用單一係數會新舊混算。
    SELECT lq.security_code,
        SUM(lq.earnings_per_share / COALESCE(ar.factor, 1)) AS ttm_eps,
        MAX(lq.net_asset_value_per_share / COALESCE(ar.factor, 1)) FILTER (WHERE lq.rn = 1) AS nav
    FROM latest_quarters lq
    LEFT JOIN action_ranges ar ON ar.stock_symbol = lq.security_code
        AND lq.period_end < ar.to_date AND (ar.from_date IS NULL OR lq.period_end >= ar.from_date)
    WHERE lq.rn <= 4
    GROUP BY lq.security_code
),
valuation_base AS (
    -- 彙整計算各估值模型所需的所有原始數值（每股數字皆已還原成現在的股數基準）
    SELECT
        s.stock_symbol, dq."Date" as q_date, dq."ClosingPrice" as q_close, ds.y_count,
        ds.p_cheap, ds.p_fair, ds.p_expensive,
        -- 1. 股利法：以歷史平均股利分別乘以 15 / 20 / 25 倍作為估值區間
        (COALESCE(ad_avg.avg_div, 0) * 15) as div_c, (COALESCE(ad_avg.avg_div, 0) * 20) as div_f, (COALESCE(ad_avg.avg_div, 0) * 25) as div_e,
        -- 2. 預估股利（欄位沿用 eps_*）：「近四季 EPS x 預期配發率」算出預估股利，再乘以 15 / 20 / 25 倍。
        --    本質與股利法同為殖利率法，只供參考、不計入加權，否則殖利率會重複計權而低估低配發率的成長股。
        CASE WHEN b.ttm_eps > 0 THEN b.ttm_eps * (fd.payout_ratio / 100.0) * 15 ELSE 0 END as eps_c,
        CASE WHEN b.ttm_eps > 0 THEN b.ttm_eps * (fd.payout_ratio / 100.0) * 20 ELSE 0 END as eps_f,
        CASE WHEN b.ttm_eps > 0 THEN b.ttm_eps * (fd.payout_ratio / 100.0) * 25 ELSE 0 END as eps_e,
        -- 3. PBR 法：以當前淨值分別乘以歷史 PBR 位階（便宜/合理/昂貴）
        (ds.pbr_low * b.nav) as pbr_c, (ds.pbr_mid * b.nav) as pbr_f, (ds.pbr_high * b.nav) as pbr_e,
        -- 4. PER 法（本益比河流圖）：近四季 EPS 乘以歷史 PER 位階。資料庫的本益比是以近四季 EPS 算出，
        --    倍數與基準必須同一口徑；舊版用 6 年平均 EPS，成長股會被嚴重低估（台積電合理價 1,063 → 2,155）。
        CASE WHEN b.ttm_eps > 0 AND ds.pe_days >= 250 THEN ds.pe_low * b.ttm_eps ELSE 0 END as per_c,
        CASE WHEN b.ttm_eps > 0 AND ds.pe_days >= 250 THEN ds.pe_mid * b.ttm_eps ELSE 0 END as per_f,
        CASE WHEN b.ttm_eps > 0 AND ds.pe_days >= 250 THEN ds.pe_high * b.ttm_eps ELSE 0 END as per_e
    FROM stocks s
    JOIN "DailyQuotes" dq ON s.stock_symbol = dq."stock_symbol" AND dq."Date" = $1
    JOIN daily_stats ds ON s.stock_symbol = ds."stock_symbol"
    JOIN final_dpr fd ON s.stock_symbol = fd.stock_symbol
    LEFT JOIN (SELECT security_code, AVG(annual_sum) as avg_div FROM adjusted_annual_dividend GROUP BY security_code) ad_avg ON s.stock_symbol = ad_avg.security_code
    LEFT JOIN per_share_basis psb ON s.stock_symbol = psb.security_code
    -- 沒有近兩年季報時沿用股票主檔的值（與改版前相同）
    CROSS JOIN LATERAL (
        SELECT COALESCE(psb.ttm_eps, s.last_four_eps) AS ttm_eps,
               COALESCE(psb.nav, s.net_asset_value_per_share) AS nav
    ) b
    WHERE ($4::varchar IS NULL OR s.stock_symbol = $4)
)
SELECT
    stock_symbol, q_date,
    -- 計算「收盤價相對於加權便宜價」的百分比，數值越低代表股價越具備吸引力
    CASE WHEN calc.weighted_cheap > 0 THEN ROUND(((q_close / calc.weighted_cheap) * 100)::numeric, 4) ELSE 0 END as percentage,
    ROUND(q_close::numeric, 4) as q_close,
    -- 輸出加權後的終極估值區間
    ROUND(calc.weighted_cheap::numeric, 4) as cheap,
    ROUND(calc.weighted_fair::numeric, 4) as fair,
    ROUND(calc.weighted_expensive::numeric, 4) as expensive,
    -- 輸出各個單獨模型的估值結果供前端報表分析（PER 為未套上限的原值）
    ROUND(COALESCE(p_cheap, 0)::numeric, 4) as price_cheap, ROUND(COALESCE(p_fair, 0)::numeric, 4) as price_fair, ROUND(COALESCE(p_expensive, 0)::numeric, 4) as price_expensive,
    ROUND(COALESCE(div_c, 0)::numeric, 4) as dividend_cheap, ROUND(COALESCE(div_f, 0)::numeric, 4) as dividend_fair, ROUND(COALESCE(div_e, 0)::numeric, 4) as dividend_expensive,
    y_count as year_count,
    ROUND(COALESCE(eps_c, 0)::numeric, 4) as eps_cheap, ROUND(COALESCE(eps_f, 0)::numeric, 4) as eps_fair, ROUND(COALESCE(eps_e, 0)::numeric, 4) as eps_expensive,
    ROUND(COALESCE(pbr_c, 0)::numeric, 4) as pbr_cheap, ROUND(COALESCE(pbr_f, 0)::numeric, 4) as pbr_fair, ROUND(COALESCE(pbr_e, 0)::numeric, 4) as pbr_expensive,
    ROUND(COALESCE(per_c, 0)::numeric, 4) as per_cheap, ROUND(COALESCE(per_f, 0)::numeric, 4) as per_fair, ROUND(COALESCE(per_e, 0)::numeric, 4) as per_expensive,
    NOW() as update_time
FROM valuation_base vb
CROSS JOIN LATERAL (
    -- 各方法的便宜/合理/昂貴三個價位都 > 0 才算有效。
    SELECT
        (COALESCE(p_cheap, 0) > 0 AND COALESCE(p_fair, 0) > 0 AND COALESCE(p_expensive, 0) > 0) AS v_price,
        (div_c > 0 AND div_f > 0 AND div_e > 0) AS v_div,
        (COALESCE(pbr_c, 0) > 0 AND COALESCE(pbr_f, 0) > 0 AND COALESCE(pbr_e, 0) > 0) AS v_pbr,
        (per_c > 0 AND per_f > 0 AND per_e > 0) AS v_per
) v
CROSS JOIN LATERAL (
    -- PER 上限基準：所有有效方法（價格、股利、PBR、PER）同一價位的中位數。含 PER 本身才不會被
    -- 看歷史的價格法、股利法拉低而誤截成長股（台積電 PER 2,156 對 其他三法中位數 625）。
    SELECT
        (SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) FROM (VALUES
            (CASE WHEN v.v_price THEN p_cheap::float8 END), (CASE WHEN v.v_div THEN div_c::float8 END),
            (CASE WHEN v.v_pbr THEN pbr_c::float8 END), (CASE WHEN v.v_per THEN per_c::float8 END)
        ) t(x) WHERE x IS NOT NULL) AS med_c,
        (SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) FROM (VALUES
            (CASE WHEN v.v_price THEN p_fair::float8 END), (CASE WHEN v.v_div THEN div_f::float8 END),
            (CASE WHEN v.v_pbr THEN pbr_f::float8 END), (CASE WHEN v.v_per THEN per_f::float8 END)
        ) t(x) WHERE x IS NOT NULL) AS med_f,
        (SELECT PERCENTILE_CONT(0.5) WITHIN GROUP (ORDER BY x) FROM (VALUES
            (CASE WHEN v.v_price THEN p_expensive::float8 END), (CASE WHEN v.v_div THEN div_e::float8 END),
            (CASE WHEN v.v_pbr THEN pbr_e::float8 END), (CASE WHEN v.v_per THEN per_e::float8 END)
        ) t(x) WHERE x IS NOT NULL) AS med_e
) md
CROSS JOIN LATERAL (
    -- 計入加權的 PER：不超過所有有效方法中位數的 3 倍，抑制景氣循環股獲利高峰時的高估
    SELECT
        CASE WHEN md.med_c IS NULL THEN per_c ELSE LEAST(per_c, 3 * md.med_c) END AS per_c_w,
        CASE WHEN md.med_f IS NULL THEN per_f ELSE LEAST(per_f, 3 * md.med_f) END AS per_f_w,
        CASE WHEN md.med_e IS NULL THEN per_e ELSE LEAST(per_e, 3 * md.med_e) END AS per_e_w
) pw
CROSS JOIN LATERAL (
    -- 權重：價格 20% + 股利 25% + PBR 20% + PER 35%。缺項不以 0 參與加權——舊版把無股利、虧損的方法
    -- 當 0 計入，548 檔（27%）的估價被整體壓低；改為只在有效方法之間按比例重新分配權重。
    SELECT
        v.v_price::int + v.v_div::int + v.v_pbr::int + v.v_per::int AS method_count,
        0.2 * v.v_price::int + 0.25 * v.v_div::int + 0.2 * v.v_pbr::int + 0.35 * v.v_per::int AS weight_sum
) m
CROSS JOIN LATERAL (
    -- 有效方法少於 2 個時不給估價：加權三價與 percentage 都寫 0（percentage 欄位 NOT NULL）。
    SELECT
        CASE WHEN m.method_count >= 2 THEN (
            0.2 * CASE WHEN v.v_price THEN p_cheap ELSE 0 END + 0.25 * CASE WHEN v.v_div THEN div_c ELSE 0 END
            + 0.2 * CASE WHEN v.v_pbr THEN pbr_c ELSE 0 END + 0.35 * CASE WHEN v.v_per THEN pw.per_c_w ELSE 0 END
        ) / m.weight_sum ELSE 0 END as weighted_cheap,
        CASE WHEN m.method_count >= 2 THEN (
            0.2 * CASE WHEN v.v_price THEN p_fair ELSE 0 END + 0.25 * CASE WHEN v.v_div THEN div_f ELSE 0 END
            + 0.2 * CASE WHEN v.v_pbr THEN pbr_f ELSE 0 END + 0.35 * CASE WHEN v.v_per THEN pw.per_f_w ELSE 0 END
        ) / m.weight_sum ELSE 0 END as weighted_fair,
        CASE WHEN m.method_count >= 2 THEN (
            0.2 * CASE WHEN v.v_price THEN p_expensive ELSE 0 END + 0.25 * CASE WHEN v.v_div THEN div_e ELSE 0 END
            + 0.2 * CASE WHEN v.v_pbr THEN pbr_e ELSE 0 END + 0.35 * CASE WHEN v.v_per THEN pw.per_e_w ELSE 0 END
        ) / m.weight_sum ELSE 0 END as weighted_expensive
) calc
ON CONFLICT (date, security_code) DO UPDATE SET
    -- 若該日期與代號已存在，則更新所有估值指標至最新計算結果
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

#[cfg(test)]
mod tests {
    use crate::infra::cache::SHARE;
    use chrono::Datelike;

    use super::*;

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 Estimate::upsert");

        let current_date = NaiveDate::parse_from_str("2023-09-15", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        let estimate = Estimate::new("9921".to_string(), current_date);

        match estimate.upsert(years_str).await {
            Ok(r) => tracing::debug!("Estimate::upsert:{:#?}", r),
            Err(why) => {
                tracing::debug!("Failed to Estimate::upsert because {:?}", why);
            }
        }

        tracing::debug!("結束 Estimate::upsert");
    }

    #[tokio::test]
    #[cfg_attr(
        not(feature = "integration-tests"),
        ignore = "需要外部服務（PostgreSQL/Redis），請加 --features integration-tests 執行"
    )]
    async fn test_upsert_all() {
        dotenvy::dotenv().ok();
        SHARE.load().await;
        tracing::debug!("開始 Estimate::upsert_all");

        let current_date = NaiveDate::parse_from_str("2026-03-03", "%Y-%m-%d").unwrap();
        let years: Vec<i32> = (0..10).map(|i| current_date.year() - i).collect();
        let years_vec: Vec<String> = years.iter().map(|&year| year.to_string()).collect();
        let years_str = years_vec.join(",");
        // 分桶結果必須與不分桶一致：兩者都是 upsert，影響筆數即輸出的股票數。
        let unbucketed = run_estimate_upsert(current_date, &years_str, None, (1, 0))
            .await
            .expect("不分桶重建應成功");
        let bucketed = Estimate::upsert_all(current_date, years_str)
            .await
            .expect("分桶重建應成功");
        assert_eq!(bucketed.rows_affected(), unbucketed.rows_affected());

        tracing::debug!("結束 Estimate::upsert_all");
    }
}
