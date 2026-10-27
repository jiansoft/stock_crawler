-- 集保股權分散週摘要加上 400 張以上欄位（2026-10-27）。
--
-- 千張的門檻對高價股太嚴（2,000 元的股票 1,000 張就是 20 億），加上 400 張以上（第 12～15 級）
-- 的人數、股數與比例。集保週由 holder_distribution_level 同一份資料算出；2026-10-02 以前的週由
-- 神秘金字塔（norway.twsthr.info）一次性回補，股數是網站的張數 × 1000。
--
-- 部署順序：
--   1. 在正式庫執行本檔（只加可為 NULL 的欄位，不重寫資料表，瞬間完成）。
--   2. 部署新版 stock_crawler：週六 10:40 入庫時一併寫入這三欄。
--   3. 以新版程式跑 test_save_holder_distributions 補上最新一週。

ALTER TABLE holder_distribution
    ADD COLUMN IF NOT EXISTS holders_400 BIGINT,
    ADD COLUMN IF NOT EXISTS shares_400  BIGINT,
    ADD COLUMN IF NOT EXISTS percent_400 NUMERIC(8, 4);

COMMENT ON TABLE holder_distribution IS '集保戶股權分散表週摘要：每週六公布上週五的資料（集保開放資料只有最新一週；2026-10-02 以前的週由神秘金字塔 norway.twsthr.info 一次性回補）';
COMMENT ON COLUMN holder_distribution.holders_400 IS '400 張以上（400,001 股以上，第 12～15 級）人數；NULL 為沒有分級資料';
COMMENT ON COLUMN holder_distribution.shares_400 IS '400 張以上股數（集保週為精確股數；回補週為網站的張數 × 1000）；NULL 為沒有分級資料';
COMMENT ON COLUMN holder_distribution.percent_400 IS '400 張以上持股佔集保庫存比例（%，由股數除以合計股數重算）；NULL 為沒有分級資料';
