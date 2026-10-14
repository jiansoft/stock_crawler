-- 每日相對強度（RS，2026-10-15）。
--
-- 由 stock_crawler 收盤回填未來報酬之後寫入：當天上市櫃股票（不含 ETF、ETN、受益證券）的
-- 加權報酬（0.4 × 近 60 日 ＋ 0.3 × 近 120 日 ＋ 0.3 × 近 240 日，還原權值）、百分位分數與名次。
-- 同一天重算時整天替換；個股相對強度歷史與訊號績效的 RS 條件讀這張表。
CREATE TABLE IF NOT EXISTS daily_relative_strength
(
    date            DATE                                   NOT NULL, -- 交易日
    stock_symbol    VARCHAR(24)                            NOT NULL, -- 股票代號
    weighted_return NUMERIC(12, 4)                         NOT NULL, -- 加權報酬（%）
    rs_score        SMALLINT                               NOT NULL, -- 相對強度分數 1–99
    rank            INTEGER                                NOT NULL, -- 名次（同分同名次）
    universe        INTEGER                                NOT NULL, -- 當天計分的股票總數
    created_at      TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (date, stock_symbol)
);

-- 依股票查歷史。
CREATE INDEX IF NOT EXISTS "daily_relative_strength-stock_symbol-date-idx"
    ON daily_relative_strength (stock_symbol, date DESC);

COMMENT ON TABLE daily_relative_strength IS '每日相對強度（上市櫃股票不含 ETF 的加權報酬百分位；收盤回填未來報酬後寫入）';
COMMENT ON COLUMN daily_relative_strength.date IS '交易日';
COMMENT ON COLUMN daily_relative_strength.stock_symbol IS '股票代號';
COMMENT ON COLUMN daily_relative_strength.weighted_return IS '加權報酬（%）：0.4 × 近 60 日 + 0.3 × 近 120 日 + 0.3 × 近 240 日，還原權值';
COMMENT ON COLUMN daily_relative_strength.rs_score IS '相對強度分數 1–99：比它弱的家數占其他家數的百分位，99 最強';
COMMENT ON COLUMN daily_relative_strength.rank IS '名次：比它強的家數 + 1，同分同名次';
COMMENT ON COLUMN daily_relative_strength.universe IS '當天計分的股票總數';
COMMENT ON COLUMN daily_relative_strength.created_at IS '寫入時間';
