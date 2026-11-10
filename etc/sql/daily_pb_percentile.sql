-- 每日股價淨值比歷史分位（財務大腕的便宜價，2026-11-10）。
--
-- 由 stock_crawler 收盤寫入風險指標之後計算：上市櫃每檔以截至當天的近 5 年（日曆年）官方股價淨值比
-- （"DailyQuotes"."price-to-book_ratio"，大於 0 才算）算當天的百分位（比當天低的天數 ÷ (樣本數 − 1)，
-- 0 為 5 年最低、100 為 5 年最高）；樣本不足 480 筆（約 2 年）不寫入。同一段日期重算時整段替換；
-- 條件選股與回測的 max_pb_percentile 與依淨值比分位排序讀這張表。
CREATE TABLE IF NOT EXISTS daily_pb_percentile
(
    date          DATE                                   NOT NULL, -- 交易日
    stock_symbol  VARCHAR(24)                            NOT NULL, -- 股票代號
    pb            NUMERIC(10, 2)                         NOT NULL, -- 當天股價淨值比（倍）
    pb_percentile NUMERIC(5, 2)                          NOT NULL, -- 當天股價淨值比在近 5 年的百分位（0–100）
    samples       SMALLINT                               NOT NULL, -- 樣本筆數（近 5 年有股價淨值比的日線）
    created_at    TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (date, stock_symbol)
);

-- 依股票查最新一筆（條件選股）。
CREATE INDEX IF NOT EXISTS "daily_pb_percentile-stock_symbol-date-idx"
    ON daily_pb_percentile (stock_symbol, date DESC);

COMMENT ON TABLE daily_pb_percentile IS '每日股價淨值比歷史分位（當天股價淨值比在自身近 5 年的百分位；收盤寫入風險指標後計算）';
COMMENT ON COLUMN daily_pb_percentile.date IS '交易日';
COMMENT ON COLUMN daily_pb_percentile.stock_symbol IS '股票代號';
COMMENT ON COLUMN daily_pb_percentile.pb IS '當天官方股價淨值比（倍）';
COMMENT ON COLUMN daily_pb_percentile.pb_percentile IS '百分位（0–100）：近 5 年比當天低的天數 ÷ (樣本數 − 1) × 100，0 為 5 年最低';
COMMENT ON COLUMN daily_pb_percentile.samples IS '樣本筆數（近 5 年有股價淨值比的日線列數，至少 480）';
COMMENT ON COLUMN daily_pb_percentile.created_at IS '寫入時間';
