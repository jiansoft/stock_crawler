-- 策略換股紀錄（2026-10-20）。
--
-- 22:10 策略換股通知遇到換股日時，把當天通知的持股（策略部位的代號、名稱、權重）與條件存下來；
-- /api/v1/strategy-presets/{name}/track 依這些紀錄逐期計算實際表現並與 0050 比較。紀錄只寫一次
-- （同策略同一天已有就保留），之後重跑回測或修改策略條件都不會改寫歷史。
CREATE TABLE IF NOT EXISTS strategy_rebalance
(
    preset_name     VARCHAR(64)                                        NOT NULL, -- 策略名稱
    date            DATE                                               NOT NULL, -- 換股日（收盤買進）
    query           TEXT                                               NOT NULL, -- 當時的回測條件
    hold_days       SMALLINT                                           NOT NULL, -- 持有天數（交易日）
    etf_weight      SMALLINT                 DEFAULT 0                 NOT NULL, -- 0050 核心部位（%）
    cash            DOUBLE PRECISION         DEFAULT 0                 NOT NULL, -- 策略部位現金比例（%）
    market_filtered BOOLEAN                  DEFAULT FALSE             NOT NULL, -- 大盤濾網沒過、策略部位空手
    symbols         TEXT[]                   DEFAULT '{}'              NOT NULL, -- 持股代號
    names           TEXT[]                   DEFAULT '{}'              NOT NULL, -- 持股名稱（與 symbols 對應）
    weights         DOUBLE PRECISION[]       DEFAULT '{}'              NOT NULL, -- 策略部位權重（%，與 symbols 對應）
    created_at      TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    PRIMARY KEY (preset_name, date)
);

COMMENT ON TABLE strategy_rebalance IS '策略換股紀錄：換股通知當天的持股，用來追蹤策略的實際表現';
COMMENT ON COLUMN strategy_rebalance.preset_name IS '策略名稱（strategy_preset.name；策略刪除後紀錄保留）';
COMMENT ON COLUMN strategy_rebalance.date IS '換股日（收盤買進）';
COMMENT ON COLUMN strategy_rebalance.query IS '當時的回測條件（query string）';
COMMENT ON COLUMN strategy_rebalance.hold_days IS '持有天數（交易日）';
COMMENT ON COLUMN strategy_rebalance.etf_weight IS '0050 核心部位（%，0 為純策略）';
COMMENT ON COLUMN strategy_rebalance.cash IS '策略部位放現金的比例（%）';
COMMENT ON COLUMN strategy_rebalance.market_filtered IS '大盤濾網沒過、策略部位空手';
COMMENT ON COLUMN strategy_rebalance.symbols IS '策略部位持股代號，依排序由前到後';
COMMENT ON COLUMN strategy_rebalance.names IS '持股名稱，與 symbols 一一對應';
COMMENT ON COLUMN strategy_rebalance.weights IS '持股在策略部位的權重（%），與 symbols 一一對應';
COMMENT ON COLUMN strategy_rebalance.created_at IS '寫入時間';
