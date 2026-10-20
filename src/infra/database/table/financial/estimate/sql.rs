//! 估值重建 SQL（`estimate` 表的 upsert），獨立成檔以免淹沒主檔的流程與說明。

/// 估值重建 SQL，公式說明見 [`super::Estimate::upsert_all`]。
///
/// 參數：`$1` 估值日期、`$2` 逗號分隔年份、`$3` 排除的 ETF 產業代碼、`$4` 單一股票代號（NULL 為全市場）、
/// `$5` 桶數、`$6` 桶號（`$5 = 1` 為不分桶）。
pub(super) const ESTIMATE_UPSERT_SQL: &str = r#"
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
