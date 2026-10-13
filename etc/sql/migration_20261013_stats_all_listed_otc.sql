-- 統計表 all 列（stock_exchange_market_id = 0）改為只算上市＋上櫃（2026-10-13）。
--
-- 舊版 all 列是全部市場加總，混入興櫃（5）與公開發行／市場別 0（未歸類 ETN 等），
-- 和 Data API 各排行、選股的 all 口徑（上市＋上櫃）不一致。本檔以同日上市（2）與上櫃（4）
-- 兩列相加重寫歷史 all 列，只改統計欄位、不動 created_at／updated_at；可重複執行。
-- 部署順序：本檔與新版 stock_crawler 先後皆可（新版寫入的 all 列本來就等於兩列相加）。

UPDATE daily_stock_price_stats t
SET
    undervalued = s.undervalued,
    fair_valued = s.fair_valued,
    overvalued = s.overvalued,
    highly_overvalued = s.highly_overvalued,
    below_5_day_moving_average = s.below_5_day_moving_average,
    above_5_day_moving_average = s.above_5_day_moving_average,
    below_20_day_moving_average = s.below_20_day_moving_average,
    above_20_day_moving_average = s.above_20_day_moving_average,
    below_60_day_moving_average = s.below_60_day_moving_average,
    above_60_day_moving_average = s.above_60_day_moving_average,
    below_120_day_moving_average = s.below_120_day_moving_average,
    above_120_day_moving_average = s.above_120_day_moving_average,
    below_240_day_moving_average = s.below_240_day_moving_average,
    above_240_day_moving_average = s.above_240_day_moving_average,
    stocks_up = s.stocks_up,
    stocks_down = s.stocks_down,
    stocks_unchanged = s.stocks_unchanged,
    new_52_week_high = s.new_52_week_high,
    new_52_week_low = s.new_52_week_low
FROM (SELECT date,
           SUM(undervalued)::int AS undervalued,
           SUM(fair_valued)::int AS fair_valued,
           SUM(overvalued)::int AS overvalued,
           SUM(highly_overvalued)::int AS highly_overvalued,
           SUM(below_5_day_moving_average)::int AS below_5_day_moving_average,
           SUM(above_5_day_moving_average)::int AS above_5_day_moving_average,
           SUM(below_20_day_moving_average)::int AS below_20_day_moving_average,
           SUM(above_20_day_moving_average)::int AS above_20_day_moving_average,
           SUM(below_60_day_moving_average)::int AS below_60_day_moving_average,
           SUM(above_60_day_moving_average)::int AS above_60_day_moving_average,
           SUM(below_120_day_moving_average)::int AS below_120_day_moving_average,
           SUM(above_120_day_moving_average)::int AS above_120_day_moving_average,
           SUM(below_240_day_moving_average)::int AS below_240_day_moving_average,
           SUM(above_240_day_moving_average)::int AS above_240_day_moving_average,
           SUM(stocks_up)::int AS stocks_up,
           SUM(stocks_down)::int AS stocks_down,
           SUM(stocks_unchanged)::int AS stocks_unchanged,
           SUM(new_52_week_high)::int AS new_52_week_high,
           SUM(new_52_week_low)::int AS new_52_week_low
      FROM daily_stock_price_stats
      WHERE stock_exchange_market_id IN (2, 4)
      GROUP BY date) s
WHERE t.date = s.date
  AND t.stock_exchange_market_id = 0;
