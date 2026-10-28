-- 每日大盤未來報酬彙總（2026-10-29）。
--
-- 由 daily_forward_return 依基準日與市場彙總：筆數、報酬總和與上漲筆數。訊號績效、法人連買績效的
-- 大盤基準與 /market/signals/performance 的 benchmark 原本每次都從 daily_forward_return 逐列 AVG
-- （三年約 400 萬列），改讀這張表。存總和與筆數而不是平均：同市場當天平均 = sum / n，區間平均 =
-- Σsum / Σn，與逐列 AVG（numeric）完全相同。
-- stock_crawler 寫入某基準日的未來報酬後重算該日；歷史以 test_backfill_market_forward_returns 回補。
CREATE TABLE IF NOT EXISTS daily_market_forward_return
(
    date      DATE     NOT NULL, -- 基準日
    market_id SMALLINT NOT NULL, -- 0 為上市櫃全部，其餘為 stocks.stock_exchange_market_id
    n5        BIGINT   NOT NULL, -- 有 5 日報酬的股票數
    sum5      NUMERIC,           -- 5 日報酬總和（%）；n5 為 0 時 NULL
    win5      NUMERIC,           -- 5 日報酬 > 0 的檔數（numeric，與逐列 AVG(1.0／0.0) 同口徑）
    n20       BIGINT   NOT NULL,
    sum20     NUMERIC,
    win20     NUMERIC,
    n60       BIGINT   NOT NULL,
    sum60     NUMERIC,
    win60     NUMERIC,
    n120      BIGINT   NOT NULL,
    sum120    NUMERIC,
    win120    NUMERIC,
    n240      BIGINT   NOT NULL,
    sum240    NUMERIC,
    win240    NUMERIC,
    updated_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (date, market_id)
);

COMMENT ON TABLE daily_market_forward_return IS '每日大盤未來報酬彙總：依基準日與市場彙總 daily_forward_return 的筆數、報酬總和與上漲檔數';
COMMENT ON COLUMN daily_market_forward_return.date IS '基準日';
COMMENT ON COLUMN daily_market_forward_return.market_id IS '0 為上市櫃全部，其餘為 stocks.stock_exchange_market_id';
COMMENT ON COLUMN daily_market_forward_return.n5 IS '有 5 日報酬的股票數（另有 20／60／120／240 日同口徑欄位）';
COMMENT ON COLUMN daily_market_forward_return.sum5 IS '5 日報酬總和（%）；平均 = sum5 / n5';
COMMENT ON COLUMN daily_market_forward_return.win5 IS '5 日報酬大於 0 的檔數；上漲比例 = win5 / n5';
