-- 市場廣度新增 52 週新高／新低家數（2026-10-13）。
--
-- 部署順序：先在正式庫執行本檔，再部署新版 stock_crawler（新版 upsert 與 Data API 會讀寫這兩欄）。
-- 回補規則與 DailyStockPriceStats::upsert 相同：當日最高價高於前 52 週（364 天、不含當日）
-- 最高價算新高、當日最低價低於前 52 週最低價算新低；前 52 週不足 200 個交易日者不計。
-- 只更新這兩欄，不動其他統計值與 updated_at；可重複執行。

ALTER TABLE daily_stock_price_stats
    ADD COLUMN IF NOT EXISTS new_52_week_high INT DEFAULT 0 NOT NULL,
    ADD COLUMN IF NOT EXISTS new_52_week_low  INT DEFAULT 0 NOT NULL;

COMMENT ON COLUMN daily_stock_price_stats.new_52_week_high IS '當日最高價高於前 52 週（364 天）最高價的股票數量；前 52 週不足 200 個交易日者不計';
COMMENT ON COLUMN daily_stock_price_stats.new_52_week_low IS '當日最低價低於前 52 週（364 天）最低價的股票數量；前 52 週不足 200 個交易日者不計';

DO
$$
    DECLARE
        stats_date DATE;
    BEGIN
        FOR stats_date IN SELECT DISTINCT date FROM daily_stock_price_stats ORDER BY date
            LOOP
                WITH raw_data AS (SELECT s.stock_exchange_market_id AS market_id,
                                         dq."HighestPrice"          AS day_high,
                                         dq."LowestPrice"           AS day_low,
                                         w.prior_high,
                                         w.prior_low,
                                         w.prior_days
                                  FROM stocks s
                                           JOIN estimate e
                                                ON s."SuspendListing" = FALSE AND s.stock_symbol = e.security_code
                                           JOIN "DailyQuotes" dq
                                                ON e.date = dq."Date" AND e.security_code = dq.stock_symbol
                                           CROSS JOIN LATERAL (SELECT MAX(p."HighestPrice") AS prior_high,
                                                                      MIN(p."LowestPrice") FILTER (WHERE p."LowestPrice" > 0) AS prior_low,
                                                                      COUNT(*) AS prior_days
                                                               FROM "DailyQuotes" p
                                                               WHERE p.stock_symbol = dq.stock_symbol
                                                                 AND p."Date" < dq."Date"
                                                                 AND p."Date" >= dq."Date" - 364
                                                                 AND p."ClosingPrice" > 0) w
                                  WHERE e.date = stats_date),
                     market_metrics AS (SELECT market_id,
                                               COUNT(*) FILTER (WHERE prior_days >= 200 AND day_high > prior_high) AS new_high,
                                               COUNT(*) FILTER (WHERE prior_days >= 200 AND day_low > 0 AND day_low < prior_low) AS new_low
                                        FROM raw_data
                                        GROUP BY market_id),
                     final_set AS (SELECT 0 AS market_id,
                                          COALESCE(SUM(new_high), 0)::int AS new_high,
                                          COALESCE(SUM(new_low), 0)::int  AS new_low
                                   FROM market_metrics
                                   UNION ALL
                                   SELECT market_id, new_high::int, new_low::int
                                   FROM market_metrics
                                   WHERE market_id != 0)
                UPDATE daily_stock_price_stats t
                SET new_52_week_high = f.new_high,
                    new_52_week_low  = f.new_low
                FROM final_set f
                WHERE t.date = stats_date
                  AND t.stock_exchange_market_id = f.market_id;
            END LOOP;
    END
$$;
