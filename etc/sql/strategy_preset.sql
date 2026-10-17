-- 具名策略（2026-10-17）。
--
-- 把一組條件選股回測的 query string（與 /api/v1/stocks/screen/backtest 相同）存成名稱，回測、偏移穩健度、
-- 前緣、歸因與滾動前推都可用 preset=名稱 引用（請求另帶的參數覆蓋存的值）；alert 為 true 的策略由
-- 22:10 的策略換股通知逐一執行。由手動回補管理介面的 /api/strategy-presets 新增、修改與刪除。
CREATE TABLE IF NOT EXISTS strategy_preset
(
    name        VARCHAR(64)                                        NOT NULL PRIMARY KEY, -- 策略名稱
    query       TEXT                                               NOT NULL,             -- 回測 query string
    description TEXT                     DEFAULT ''                NOT NULL,             -- 說明
    alert       BOOLEAN                  DEFAULT FALSE             NOT NULL,             -- 是否發換股通知
    created_at  TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL,
    updated_at  TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

COMMENT ON TABLE strategy_preset IS '具名策略：條件選股回測的 query string，可用 preset=名稱 引用；alert 為 true 時發換股通知';
COMMENT ON COLUMN strategy_preset.name IS '策略名稱（1–64 字元，不含 & = # ? 與前後空白）';
COMMENT ON COLUMN strategy_preset.query IS '條件選股回測的 query string（不含 preset）';
COMMENT ON COLUMN strategy_preset.description IS '說明';
COMMENT ON COLUMN strategy_preset.alert IS '是否在 22:10 發策略換股通知（query 必須含 from）';
COMMENT ON COLUMN strategy_preset.created_at IS '建立時間';
COMMENT ON COLUMN strategy_preset.updated_at IS '最後修改時間';
