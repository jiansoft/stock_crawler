
CREATE TABLE daily_stock_price_stats
(
    date                         DATE                     DEFAULT CURRENT_DATE NOT NULL, -- 統計日期
    stock_exchange_market_id     INT                                           NOT NULL, -- 市場類型 (TWSE: 2, TPEx: 4, ALL: 0)
    undervalued                  INT                      DEFAULT 0            NOT NULL, -- 股價 <= 便宜價的股票數量
    fair_valued                  INT                      DEFAULT 0            NOT NULL, -- 便宜價 < 股價 <= 合理價的股票數量
    overvalued                   INT                      DEFAULT 0            NOT NULL, -- 合理價 < 股價 <= 昂貴價的股票數量
    highly_overvalued            INT                      DEFAULT 0            NOT NULL, -- 股價 > 昂貴價的股票數量
    below_5_day_moving_average   INT                      DEFAULT 0            NOT NULL, -- 股價 < 月線的股票數量
    above_5_day_moving_average   INT                      DEFAULT 0            NOT NULL, -- 股價 >= 月線的股票數量
    below_20_day_moving_average  INT                      DEFAULT 0            NOT NULL, -- 股價 < 月線的股票數量
    above_20_day_moving_average  INT                      DEFAULT 0            NOT NULL, -- 股價 >= 月線的股票數量
    below_60_day_moving_average  INT                      DEFAULT 0            NOT NULL, -- 股價 < 季線的股票數量
    above_60_day_moving_average  INT                      DEFAULT 0            NOT NULL, -- 股價 >= 季線的股票數量
    below_120_day_moving_average INT                      DEFAULT 0            NOT NULL, -- 股價 < 半年線的股票數量
    above_120_day_moving_average INT                      DEFAULT 0            NOT NULL, -- 股價 >= 半年線的股票數量
    below_240_day_moving_average INT                      DEFAULT 0            NOT NULL, -- 股價 < 年線的股票數量
    above_240_day_moving_average INT                      DEFAULT 0            NOT NULL, -- 股價 >= 年線的股票數量
    stocks_up                    INT                      DEFAULT 0            NOT NULL, -- 上漲的股票數量
    stocks_down                  INT                      DEFAULT 0            NOT NULL, -- 下跌的股票數量
    stocks_unchanged             INT                      DEFAULT 0            NOT NULL, -- 持平的股票數量
    created_at                   TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,     -- 記錄創建時間
    updated_at                   TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,     -- 記錄最後更新時間
    PRIMARY KEY (date, stock_exchange_market_id)
);


COMMENT ON TABLE daily_stock_price_stats IS '每日股票價格統計表';
COMMENT ON COLUMN daily_stock_price_stats.date IS '統計日期';
COMMENT ON COLUMN daily_stock_price_stats.stock_exchange_market_id IS '市場類型 (TWSE: 2, TPEx: 4, ALL: 0)';
COMMENT ON COLUMN daily_stock_price_stats.undervalued IS '股價 <= 便宜價的股票數量，股票價格低於其估計的"便宜"價格';
COMMENT ON COLUMN daily_stock_price_stats.fair_valued IS '便宜價 < 股價 <= 合理價的股票數量 股票價格處於低於合理價格的情況';
COMMENT ON COLUMN daily_stock_price_stats.overvalued IS '合理價 < 股價 <= 昂貴價的股票數量，股票價格高於其估計的"合理"價格但低於"昂貴"價格的情況';
COMMENT ON COLUMN daily_stock_price_stats.highly_overvalued IS '股價 > 昂貴價的股票數量，股票價格高於其估計的"昂貴"價格';
COMMENT ON COLUMN daily_stock_price_stats.below_5_day_moving_average IS '股價 < 周線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.above_5_day_moving_average IS '股價 >= 周線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.below_20_day_moving_average IS '股價 < 月線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.above_20_day_moving_average IS '股價 >= 月線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.below_60_day_moving_average IS '股價 < 季線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.above_60_day_moving_average IS '股價 >= 季線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.below_120_day_moving_average IS '股價 < 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.above_120_day_moving_average IS '股價 >= 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.below_240_day_moving_average IS '股價 < 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.above_240_day_moving_average IS '股價 >= 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.stocks_up IS '上漲的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.stocks_down IS '下跌的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.stocks_unchanged IS '持平的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.created_at IS '記錄創建時間';
COMMENT ON COLUMN daily_stock_price_stats.updated_at IS '記錄最後更新時間';

--CREATE INDEX idx_daily_stock_price_stats_date ON daily_stock_price_stats (date);

/*
CREATE TABLE daily_stock_price_stats
(
    date DATE DEFAULT CURRENT_DATE NOT NULL,

    all_cheap_count INT DEFAULT 0 NOT NULL,
    all_fair_count INT DEFAULT 0 NOT NULL,
    all_expensive_count INT DEFAULT 0 NOT NULL,
    all_very_expensive_count INT DEFAULT 0 NOT NULL,
    all_below_half_year_ma_count INT DEFAULT 0 NOT NULL,
    all_above_half_year_ma_count INT DEFAULT 0 NOT NULL,
    all_below_year_ma_count INT DEFAULT 0 NOT NULL,
    all_above_year_ma_count INT DEFAULT 0 NOT NULL,
    all_total_stocks INT DEFAULT 0 NOT NULL,

    twse_cheap_count INT DEFAULT 0 NOT NULL,
    twse_fair_count INT DEFAULT 0 NOT NULL,
    twse_expensive_count INT DEFAULT 0 NOT NULL,
    twse_very_expensive_count INT DEFAULT 0 NOT NULL,
    twse_below_half_year_ma_count INT DEFAULT 0 NOT NULL,
    twse_above_half_year_ma_count INT DEFAULT 0 NOT NULL,
    twse_below_year_ma_count INT DEFAULT 0 NOT NULL,
    twse_above_year_ma_count INT DEFAULT 0 NOT NULL,
    twse_total_stocks INT DEFAULT 0 NOT NULL,

    tpex_cheap_count INT DEFAULT 0 NOT NULL,
    tpex_fair_count INT DEFAULT 0 NOT NULL,
    tpex_expensive_count INT DEFAULT 0 NOT NULL,
    tpex_very_expensive_count INT DEFAULT 0 NOT NULL,
    tpex_below_half_year_ma_count INT DEFAULT 0 NOT NULL,
    tpex_above_half_year_ma_count INT DEFAULT 0 NOT NULL,
    tpex_below_year_ma_count INT DEFAULT 0 NOT NULL,
    tpex_above_year_ma_count INT DEFAULT 0 NOT NULL,
    tpex_total_stocks INT DEFAULT 0 NOT NULL,

    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (date)
);

COMMENT ON TABLE daily_stock_price_stats IS '每日股票價格統計表';
COMMENT ON COLUMN daily_stock_price_stats.date IS '統計日期';

COMMENT ON COLUMN daily_stock_price_stats.all_cheap_count IS '所有市場股價 <= 便宜價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_fair_count IS '所有市場便宜價 < 股價 <= 合理價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_expensive_count IS '所有市場合理價 < 股價 <= 昂貴價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_very_expensive_count IS '所有市場股價 > 昂貴價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_below_half_year_ma_count IS '所有市場股價 < 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_above_half_year_ma_count IS '所有市場股價 >= 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_below_year_ma_count IS '所有市場股價 < 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_above_year_ma_count IS '所有市場股價 >= 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.all_total_stocks IS '所有市場的總股票數量';

COMMENT ON COLUMN daily_stock_price_stats.twse_cheap_count IS '上市股價 <= 便宜價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_fair_count IS '上市便宜價 < 股價 <= 合理價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_expensive_count IS '上市合理價 < 股價 <= 昂貴價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_very_expensive_count IS '上市股價 > 昂貴價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_below_half_year_ma_count IS '上市股價 < 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_above_half_year_ma_count IS '上市股價 >= 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_below_year_ma_count IS '上市股價 < 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_above_year_ma_count IS '上市股價 >= 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.twse_total_stocks IS '上市市場的總股票數量';

COMMENT ON COLUMN daily_stock_price_stats.tpex_cheap_count IS '上櫃股價 <= 便宜價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_fair_count IS '上櫃便宜價 < 股價 <= 合理價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_expensive_count IS '上櫃合理價 < 股價 <= 昂貴價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_very_expensive_count IS '上櫃股價 > 昂貴價的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_below_half_year_ma_count IS '上櫃股價 < 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_above_half_year_ma_count IS '上櫃股價 >= 半年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_below_year_ma_count IS '上櫃股價 < 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_above_year_ma_count IS '上櫃股價 >= 年線的股票數量';
COMMENT ON COLUMN daily_stock_price_stats.tpex_total_stocks IS '上櫃市場的總股票數量';

COMMENT ON COLUMN daily_stock_price_stats.created_at IS '記錄創建時間';
COMMENT ON COLUMN daily_stock_price_stats.updated_at IS '記錄最後更新時間';

CREATE INDEX idx_daily_stock_price_stats_date ON daily_stock_price_stats (date);
*/
/*
WITH cte AS (
    SELECT e.serial, e.security_code, e.date, e.percentage, e.closing_price,
           e.cheap, e.fair, e.expensive, e.price_cheap, e.price_fair,
           e.price_expensive, e.dividend_cheap, e.dividend_fair, e.dividend_expensive, e.year_count,
           e.create_time, e.update_time, e.eps_cheap, e.eps_fair, e.eps_expensive,
           e.pbr_cheap, e.pbr_fair, e.pbr_expensive, e.per_cheap, e.per_fair,
           e.per_expensive, dq."Serial", dq."Date", dq."SecurityCode", dq."TradingVolume",
           dq."Transaction", dq."TradeValue", dq."OpeningPrice", dq."HighestPrice", dq."LowestPrice",
           dq."ClosingPrice", dq."ChangeRange", dq."Change", dq."LastBestBidPrice", dq."LastBestBidVolume",
           dq."LastBestAskPrice", dq."LastBestAskVolume", dq."PriceEarningRatio", dq."RecordTime", dq."CreateTime",
           dq."MovingAverage5", dq."MovingAverage10", dq."MovingAverage20", dq."MovingAverage60", dq."MovingAverage120",
           dq."MovingAverage240", dq.maximum_price_in_year, dq.minimum_price_in_year, dq.average_price_in_year,
           dq.maximum_price_in_year_date_on, dq.minimum_price_in_year_date_on, dq."price-to-book_ratio",
           s.stock_exchange_market_id, s.stock_industry_id
    FROM stocks s
    INNER JOIN estimate e ON s."SuspendListing" = false AND s.stock_symbol = e.security_code
    INNER JOIN public."DailyQuotes" dq ON e.date = dq."Date" AND e.security_code = dq."SecurityCode"
    WHERE e.date = '2024-10-01'
),
stats AS (
    SELECT
        CASE
            WHEN closing_price <= cheap THEN 'undervalued'
            WHEN closing_price > cheap AND closing_price <= fair THEN 'fair_valued'
            WHEN closing_price > fair AND closing_price <= expensive THEN 'overvalued'
            WHEN closing_price > expensive THEN  'highly_overvalued'
        END AS valuation_category,
        CASE
            WHEN closing_price <= "MovingAverage120" THEN 'below_half_year_ma'
            WHEN closing_price > "MovingAverage120" THEN 'above_half_year_ma'
        END AS ma120_category,
        CASE
            WHEN closing_price <= "MovingAverage240" THEN 'below_year_ma'
            WHEN closing_price > "MovingAverage240" THEN 'above_year_ma'
        END AS ma240_category,
         CASE
            WHEN "ChangeRange" > 0 THEN 'up'
            WHEN "ChangeRange" < 0 THEN 'down'
            WHEN "ChangeRange" = 0 THEN 'unchanged'
        END AS change_category,
        stock_exchange_market_id
    FROM cte
),
final_stats AS (
    SELECT
        0 AS market,
        COUNT(*) FILTER (WHERE valuation_category = 'undervalued') AS undervalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'fair_valued') AS fair_valued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'overvalued') AS overvalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'highly_overvalued') AS highly_overvalued_count,
        COUNT(*) FILTER (WHERE ma120_category = 'below_half_year_ma') AS below_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma120_category = 'above_half_year_ma') AS above_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'below_year_ma') AS below_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'above_year_ma') AS above_year_ma_count,
        COUNT(*) FILTER (WHERE change_category = 'up') AS up_count,
        COUNT(*) FILTER (WHERE change_category = 'down') AS down_count,
        COUNT(*) FILTER (WHERE change_category = 'unchanged') AS unchanged_count
    FROM stats
    UNION ALL
    SELECT
        2 AS market,
        COUNT(*) FILTER (WHERE valuation_category = 'undervalued' AND stock_exchange_market_id = 2) AS undervalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'fair_valued' AND stock_exchange_market_id = 2) AS fair_valued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'overvalued' AND stock_exchange_market_id = 2) AS overvalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'highly_overvalued' AND stock_exchange_market_id = 2) AS highly_overvalued_count,
        COUNT(*) FILTER (WHERE ma120_category = 'below_half_year_ma' AND stock_exchange_market_id = 2) AS below_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma120_category = 'above_half_year_ma' AND stock_exchange_market_id = 2) AS above_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'below_year_ma' AND stock_exchange_market_id = 2) AS below_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'above_year_ma' AND stock_exchange_market_id = 2) AS above_year_ma_count,
        COUNT(*) FILTER (WHERE change_category = 'up' AND stock_exchange_market_id = 2) AS up_count,
        COUNT(*) FILTER (WHERE change_category = 'down' AND stock_exchange_market_id = 2) AS down_count,
        COUNT(*) FILTER (WHERE change_category = 'unchanged' AND stock_exchange_market_id = 2) AS unchanged_count
    FROM stats
    UNION ALL
    SELECT
        4 AS market,
        COUNT(*) FILTER (WHERE valuation_category = 'undervalued' AND stock_exchange_market_id = 4) AS undervalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'fair_valued' AND stock_exchange_market_id = 4) AS fair_valued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'overvalued' AND stock_exchange_market_id = 4) AS overvalued_count,
        COUNT(*) FILTER (WHERE valuation_category = 'highly_overvalued' AND stock_exchange_market_id = 4) AS highly_overvalued_count,
        COUNT(*) FILTER (WHERE ma120_category = 'below_half_year_ma' AND stock_exchange_market_id = 4) AS below_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma120_category = 'above_half_year_ma' AND stock_exchange_market_id = 4) AS above_half_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'below_year_ma' AND stock_exchange_market_id = 4) AS below_year_ma_count,
        COUNT(*) FILTER (WHERE ma240_category = 'above_year_ma' AND stock_exchange_market_id = 4) AS above_year_ma_count,
        COUNT(*) FILTER (WHERE change_category = 'up' AND stock_exchange_market_id = 4) AS up_count,
        COUNT(*) FILTER (WHERE change_category = 'down' AND stock_exchange_market_id = 4) AS down_count,
        COUNT(*) FILTER (WHERE change_category = 'unchanged' AND stock_exchange_market_id = 4) AS unchanged_count
    FROM stats
)
INSERT INTO daily_stock_price_stats (
    date,
    stock_exchange_market_id,
    undervalued,
    fair_valued,
    overvalued,
    highly_overvalued,
    below_half_year_ma,
    above_half_year_ma,
    below_year_ma,
    above_year_ma,
    stocks_up,
    stocks_down,
    stocks_unchanged
)
SELECT
    '2024-10-01'::DATE,
    market,
    undervalued_count,
    fair_valued_count,
    overvalued_count,
    highly_overvalued_count,
    below_half_year_ma_count,
    above_half_year_ma_count,
    below_year_ma_count,
    above_year_ma_count,
    up_count,
    down_count,
    unchanged_count
FROM final_stats
ON CONFLICT (date, stock_exchange_market_id) DO UPDATE SET
    undervalued = EXCLUDED.undervalued,
    fair_valued = EXCLUDED.fair_valued,
    overvalued = EXCLUDED.overvalued,
    highly_overvalued = EXCLUDED.highly_overvalued,
    below_half_year_ma = EXCLUDED.below_half_year_ma,
    above_half_year_ma = EXCLUDED.above_half_year_ma,
    below_year_ma = EXCLUDED.below_year_ma,
    above_year_ma = EXCLUDED.above_year_ma,
    stocks_up = EXCLUDED.stocks_up,
    stocks_down = EXCLUDED.stocks_down,
    stocks_unchanged = EXCLUDED.stocks_unchanged,
    updated_at = CURRENT_TIMESTAMP;

*/