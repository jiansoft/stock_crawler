-- 每日未來報酬（還原權值，2026-10-14）。
--
-- 由 stock_crawler 收盤後回填：交易日 T 收盤時，把 T 往前第 5、20、60、120、240 個交易日（基準日）的
-- 「基準日收盤 → T 收盤」報酬寫入對應欄位，供 Data API 統計技術訊號出現後的表現；
-- 反過來看，基準日 T−N 的 N 日報酬就是 T 的近 N 日報酬，相對強度排行由此計算。
-- 報酬依除權息、減資、分割還原（除權息當天以參考價計，等同含息報酬）；尚未滿 N 個交易日為 NULL。
CREATE TABLE IF NOT EXISTS daily_forward_return
(
    date         DATE                                   NOT NULL, -- 基準日（訊號出現的交易日）
    stock_symbol VARCHAR(24)                            NOT NULL, -- 股票代號
    return_5d    NUMERIC(12, 4),                                  -- 之後 5 個交易日的報酬（%）
    return_20d   NUMERIC(12, 4),                                  -- 之後 20 個交易日的報酬（%）
    return_60d   NUMERIC(12, 4),                                  -- 之後 60 個交易日的報酬（%）
    return_120d  NUMERIC(12, 4),                                  -- 之後 120 個交易日的報酬（%）
    return_240d  NUMERIC(12, 4),                                  -- 之後 240 個交易日的報酬（%）
    updated_at   TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (date, stock_symbol)
);

-- 2026-10-14 加 120／240 日（相對強度排行用）；已建表的環境靠這兩行補欄位。
ALTER TABLE daily_forward_return ADD COLUMN IF NOT EXISTS return_120d NUMERIC(12, 4);
ALTER TABLE daily_forward_return ADD COLUMN IF NOT EXISTS return_240d NUMERIC(12, 4);

COMMENT ON TABLE daily_forward_return IS '每日未來報酬（還原權值；收盤後回填 5／20／60／120／240 個交易日前的基準日）';
COMMENT ON COLUMN daily_forward_return.date IS '基準日（訊號出現的交易日）';
COMMENT ON COLUMN daily_forward_return.stock_symbol IS '股票代號';
COMMENT ON COLUMN daily_forward_return.return_5d IS '基準日收盤到之後第 5 個交易日收盤的還原報酬（%）；未滿為 NULL';
COMMENT ON COLUMN daily_forward_return.return_20d IS '基準日收盤到之後第 20 個交易日收盤的還原報酬（%）；未滿為 NULL';
COMMENT ON COLUMN daily_forward_return.return_60d IS '基準日收盤到之後第 60 個交易日收盤的還原報酬（%）；未滿為 NULL';
COMMENT ON COLUMN daily_forward_return.return_120d IS '基準日收盤到之後第 120 個交易日收盤的還原報酬（%）；未滿為 NULL';
COMMENT ON COLUMN daily_forward_return.return_240d IS '基準日收盤到之後第 240 個交易日收盤的還原報酬（%）；未滿為 NULL';
COMMENT ON COLUMN daily_forward_return.updated_at IS '最後回填時間';
