-- 每日籌碼加上預先計算的連續買賣超天數（2026-10-15）。
--
-- 原本法人連買報酬、訊號績效／條件選股回測的籌碼條件、法人買賣超排行與持股通知，都在查詢時用
-- 視窗函數從頭數連續天數；七年期間要掃 330 萬列，查詢 4～15 秒，且各處回看長度不同（60 個交易日
-- 或 60 個日曆天）會把長連續段截斷。改成寫入時就算好存在 chip_daily。
--
-- 部署順序：
--   1. 在正式庫執行本檔（只加可為 NULL 的欄位，不重寫資料表，瞬間完成）。
--   2. 以新版程式跑 test_backfill_chip_streaks 從 2019-01-02 回補（只寫這四欄、可重複執行）。
--   3. 部署新版 stock_crawler：每日籌碼入庫後會接著更新當天的連續天數，讀取端改讀這四欄。
--
-- 計算規則見 PgChipRepository::refresh_streaks：依日期遞推，方向與前一列相同就累加，
-- 淨額 0 或缺值為 0（中斷）。

ALTER TABLE chip_daily
    ADD COLUMN IF NOT EXISTS foreign_streak SMALLINT,
    ADD COLUMN IF NOT EXISTS trust_streak   SMALLINT,
    ADD COLUMN IF NOT EXISTS dealer_streak  SMALLINT,
    ADD COLUMN IF NOT EXISTS total_streak   SMALLINT;

COMMENT ON COLUMN chip_daily.foreign_streak IS '外資截至當日的連續買賣超天數：正數連續買超、負數連續賣超、0 為當日淨額 0 或沒有資料；只數這檔有籌碼列的日子，NULL 為尚未計算';
COMMENT ON COLUMN chip_daily.trust_streak IS '投信連續買賣超天數（規則同 foreign_streak）';
COMMENT ON COLUMN chip_daily.dealer_streak IS '自營商連續買賣超天數（規則同 foreign_streak）';
COMMENT ON COLUMN chip_daily.total_streak IS '三大法人合計連續買賣超天數（規則同 foreign_streak，缺值以 0 相加）';
