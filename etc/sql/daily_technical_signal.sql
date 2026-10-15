-- 每日技術訊號（收盤後全市場預先計算，2026-10-13）。
--
-- 由 stock_crawler 收盤匯總計算上市櫃有成交股票的均線／KD／MACD 交叉、站上或跌破均線、
-- RSI／KD 超買超賣與 52 週新高新低，供 Data API `/market/signals` 查詢。
-- 每個交易日整批覆蓋（同一交易內先刪後寫）。
-- 相對強度穿越 80／50 分（indicator rs80、rs50，2026-10-15 起）由寫入 daily_relative_strength 時
-- 一併整天覆蓋；技術訊號的整天覆蓋不動這兩種來源。
CREATE TABLE IF NOT EXISTS daily_technical_signal
(
    date         DATE                                   NOT NULL, -- 交易日
    stock_symbol VARCHAR(24)                            NOT NULL, -- 股票代號
    indicator    VARCHAR(16)                            NOT NULL, -- 訊號來源，例如 ma5_ma20、kd、macd、close_ma60、rsi14、52_week、rs80
    kind         VARCHAR(16)                            NOT NULL, -- 訊號種類：golden_cross、death_cross、cross_above、cross_below、overbought、oversold、new_high、new_low
    description  TEXT                                   NOT NULL, -- 中文說明
    created_at   TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (date, stock_symbol, indicator, kind)
);

-- 個股訊號歷史（/stocks/{symbol}/signals）依代號查一段期間；主鍵以日期為首，另建代號索引。
CREATE INDEX IF NOT EXISTS daily_technical_signal_symbol_date_idx
    ON daily_technical_signal (stock_symbol, date DESC);

COMMENT ON TABLE daily_technical_signal IS '每日技術訊號（收盤後全市場預先計算；日線依股利與公司行動還原權值後計算）';
COMMENT ON COLUMN daily_technical_signal.date IS '交易日';
COMMENT ON COLUMN daily_technical_signal.stock_symbol IS '股票代號';
COMMENT ON COLUMN daily_technical_signal.indicator IS '訊號來源：ma5_ma20、ma20_ma60、kd、macd、close_ma20、close_ma60、close_ma240、rsi14、52_week、rs80、rs50（相對強度穿越 80／50 分）';
COMMENT ON COLUMN daily_technical_signal.kind IS '訊號種類：golden_cross、death_cross、cross_above、cross_below、overbought、oversold、new_high、new_low';
COMMENT ON COLUMN daily_technical_signal.description IS '中文說明';
COMMENT ON COLUMN daily_technical_signal.created_at IS '寫入時間';
