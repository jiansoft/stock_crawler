-- ETF 專區（2026-10-21）。
--
-- stock_crawler 寫入、stock_go 的 /stock/etfs 讀取：
--   etf_daily_nav   每個交易日 17:30 由證交所 MIS「ETF 預估淨值」寫入，隔一個營業日以「前一營業日單位淨值」回填 nav；
--                   MIS 上線（2026-10-21）前的日子由 MoneyDJ 歷史淨值回補（只有 nav 與市價，單位數為 NULL）
--   etf_profile     每週六由 Yahoo ETF 頁寫入；沒有 Yahoo 資料的 ETF 以代號與名稱推得的分類補一列（source = 'heuristic'）
--   etf_holding     前十大持股（只留最新一期）
--   etf_allocation  行業比重與資產分佈（只留最新一期）
--   etf_performance 以還原日線（含息）計算的期間報酬與風險指標，每個交易日重算
--   etf_snapshot    清單頁需要的欄位預先併好，每檔一列；淨值、績效、籌碼入庫後重建
CREATE TABLE IF NOT EXISTS etf_daily_nav
(
    date              DATE                                   NOT NULL, -- 資料日期（證交所 MIS 的 i 欄）
    stock_symbol      VARCHAR(24)                            NOT NULL,
    outstanding_units BIGINT,                                          -- 已發行受益權單位數；歷史回補的列為 NULL
    units_change      BIGINT,                                          -- 與前一營業日已發行單位數的差異（正為淨申購）；歷史回補的列為 NULL
    market_price      NUMERIC(18, 4),                                  -- 資料時間的成交價；無成交為 NULL
    estimated_nav     NUMERIC(18, 4),                                  -- 投信預估淨值
    premium_rate      NUMERIC(10, 4),                                  -- 預估折溢價幅度（%）
    nav               NUMERIC(18, 4),                                  -- 正式單位淨值：下一個營業日以「前一營業日單位淨值」回填
    source_time       TIMESTAMP WITH TIME ZONE,                        -- 來源資料時間
    updated_at        TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT etf_daily_nav_pkey PRIMARY KEY (date, stock_symbol)
);

CREATE INDEX IF NOT EXISTS etf_daily_nav_symbol_date_idx ON etf_daily_nav (stock_symbol, date DESC);

-- 2026-10-22：單位數改為可空，供 MoneyDJ 歷史淨值回補（已建表的資料庫也要執行）。
ALTER TABLE etf_daily_nav ALTER COLUMN outstanding_units DROP NOT NULL;
ALTER TABLE etf_daily_nav ALTER COLUMN units_change DROP NOT NULL;

COMMENT ON TABLE etf_daily_nav IS 'ETF 每日淨值、折溢價與受益單位（證交所 MIS 預估淨值）';
COMMENT ON COLUMN etf_daily_nav.units_change IS '與前一營業日已發行單位數的差異；正為淨申購、負為淨買回';
COMMENT ON COLUMN etf_daily_nav.premium_rate IS '預估折溢價幅度（%）：(成交價 - 預估淨值) / 預估淨值';
COMMENT ON COLUMN etf_daily_nav.nav IS '正式單位淨值，下一個營業日回填；當天只有 estimated_nav';

CREATE TABLE IF NOT EXISTS etf_profile
(
    stock_symbol       VARCHAR(24)                            NOT NULL,
    full_name          TEXT                                   NOT NULL DEFAULT '', -- 基金全名
    issuer             TEXT                                   NOT NULL DEFAULT '', -- 發行投信
    inception_date     DATE,                                                       -- 成立日
    benchmark          TEXT                                   NOT NULL DEFAULT '', -- 追蹤指數
    category           TEXT                                   NOT NULL DEFAULT '', -- 來源的類型（指數型、主動型…）
    is_active          BOOLEAN                                NOT NULL DEFAULT FALSE, -- 主動式
    region             TEXT                                   NOT NULL DEFAULT '', -- 投資地區（台灣、美國、全球…）
    strategy           TEXT                                   NOT NULL DEFAULT '', -- 本站分類，見下方 COMMENT
    themes             TEXT[]                                 NOT NULL DEFAULT '{}', -- 題材
    dividend_frequency TEXT                                   NOT NULL DEFAULT '', -- monthly、quarterly、semi_annual、annual、none；未知為空字串
    management_fee     TEXT                                   NOT NULL DEFAULT '', -- 經理費（來源原文，例如 0.3200%）
    custodian_fee      TEXT                                   NOT NULL DEFAULT '', -- 保管費（來源原文，可能是文字說明）
    currency           TEXT                                   NOT NULL DEFAULT '',
    exchange_name      TEXT                                   NOT NULL DEFAULT '',
    manager            TEXT                                   NOT NULL DEFAULT '', -- 經理人
    website            TEXT                                   NOT NULL DEFAULT '',
    total_assets       NUMERIC(20, 2),                                             -- 來源公布的資產規模（元）
    total_assets_date  DATE,
    nav_return         JSONB,                                                      -- 來源的淨值報酬（%）：{"date": "...", "1w": ..., "1y": ..., "10y": ...}
    manual_override    JSONB                                  NOT NULL DEFAULT '{}', -- 人工覆寫 strategy／region／themes／dividend_frequency，優先於來源
    source             TEXT                                   NOT NULL DEFAULT '', -- yahoo、heuristic
    updated_at         TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT etf_profile_pkey PRIMARY KEY (stock_symbol)
);

COMMENT ON TABLE etf_profile IS 'ETF 基本資料與分類（每週由 Yahoo ETF 頁更新；manual_override 可人工覆寫分類）';
COMMENT ON COLUMN etf_profile.strategy IS 'market_cap 市值型、high_dividend 高股息、active 主動式、bond 債券、leveraged 槓桿、inverse 反向、commodity 期貨商品、overseas 海外股票、thematic 主題';
COMMENT ON COLUMN etf_profile.manual_override IS '例如 {"strategy": "high_dividend", "themes": ["半導體"]}；有值的鍵優先於來源與推斷';
COMMENT ON COLUMN etf_profile.nav_return IS '來源公布的淨值報酬（%）；1 年以上為年化';

CREATE TABLE IF NOT EXISTS etf_holding
(
    stock_symbol   VARCHAR(24)                            NOT NULL,
    rank           SMALLINT                               NOT NULL, -- 1 起算
    as_of          DATE                                   NOT NULL, -- 持股資料日期
    holding_symbol VARCHAR(32)                            NOT NULL DEFAULT '', -- 台股成分股代號；債券等沒有代號為空字串
    holding_name   TEXT                                   NOT NULL,
    weight         NUMERIC(8, 4)                          NOT NULL, -- 比例（%）
    updated_at     TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT etf_holding_pkey PRIMARY KEY (stock_symbol, rank)
);

COMMENT ON TABLE etf_holding IS 'ETF 前十大持股（只留最新一期）';

-- 2026-10-23：前十大持股每一期都留一份，供每週市場摘要比較持有 ETF 的持股變化。
CREATE TABLE IF NOT EXISTS etf_holding_history
(
    stock_symbol   VARCHAR(24)                            NOT NULL,
    as_of          DATE                                   NOT NULL, -- 持股資料日期（一期）
    rank           SMALLINT                               NOT NULL,
    holding_symbol VARCHAR(32)                            NOT NULL DEFAULT '',
    holding_name   TEXT                                   NOT NULL,
    weight         NUMERIC(8, 4)                          NOT NULL, -- 比例（%）
    recorded_at    TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL, -- 第一次收到這一期的時間
    CONSTRAINT etf_holding_history_pkey PRIMARY KEY (stock_symbol, as_of, rank)
);

COMMENT ON TABLE etf_holding_history IS 'ETF 前十大持股的歷史，每一期一份；recorded_at 為第一次收到該期的時間';

-- 建表時把目前的最新一期帶進來（重跑安全）。
INSERT INTO etf_holding_history (stock_symbol, as_of, rank, holding_symbol, holding_name, weight)
SELECT stock_symbol, as_of, rank, holding_symbol, holding_name, weight FROM etf_holding
ON CONFLICT (stock_symbol, as_of, rank) DO NOTHING;

CREATE TABLE IF NOT EXISTS etf_allocation
(
    stock_symbol VARCHAR(24)                            NOT NULL,
    kind         VARCHAR(16)                            NOT NULL, -- industry 行業比重、asset 資產分佈
    label        TEXT                                   NOT NULL,
    as_of        DATE                                   NOT NULL,
    weight       NUMERIC(8, 4)                          NOT NULL, -- 比例（%）
    sort_order   SMALLINT                               NOT NULL, -- 來源的排列順序
    updated_at   TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT etf_allocation_pkey PRIMARY KEY (stock_symbol, kind, label)
);

COMMENT ON TABLE etf_allocation IS 'ETF 行業比重（industry）與資產分佈（asset），只留最新一期';

CREATE TABLE IF NOT EXISTS etf_performance
(
    stock_symbol VARCHAR(24)                            NOT NULL,
    calc_date    DATE                                   NOT NULL, -- 計算基準日（最後一個有成交的交易日）
    ret_1w       NUMERIC(12, 4), -- 期間報酬（%，含息、非年化）；歷史不足為 NULL
    ret_1m       NUMERIC(12, 4),
    ret_3m       NUMERIC(12, 4),
    ret_6m       NUMERIC(12, 4),
    ret_ytd      NUMERIC(12, 4),
    ret_1y       NUMERIC(12, 4), -- 1 年以上為年化報酬（%）
    ret_2y       NUMERIC(12, 4),
    ret_3y       NUMERIC(12, 4),
    ret_5y       NUMERIC(12, 4),
    ret_10y      NUMERIC(12, 4),
    vol_1y       NUMERIC(12, 4), -- 近 1 年日報酬年化標準差（%）
    mdd_1y       NUMERIC(12, 4), -- 近 1 年最大回撤（%，負值）
    mdd_date     DATE,           -- 最大回撤的谷底日
    beta_1y      NUMERIC(12, 4), -- 近 1 年對 0050 的 beta
    sharpe_1y    NUMERIC(12, 4), -- 近 1 年年化報酬 / 年化波動（未扣無風險利率）
    year_low     NUMERIC(18, 4), -- 今年還原後最低收盤
    year_high    NUMERIC(18, 4), -- 今年還原後最高收盤
    updated_at   TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT etf_performance_pkey PRIMARY KEY (stock_symbol)
);

COMMENT ON TABLE etf_performance IS 'ETF 含息報酬與近 1 年風險指標（還原日線計算，每個交易日重算）';

-- 2026-10-23：以正式淨值（現金配息還原）自算的含息報酬，取代每週更新的 Yahoo 淨值報酬（已建表的資料庫也要執行）。
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_calc_date DATE;
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_1w NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_1m NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_3m NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_6m NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_ytd NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_1y NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_2y NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_3y NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_5y NUMERIC(12, 4);
ALTER TABLE etf_performance ADD COLUMN IF NOT EXISTS nav_ret_10y NUMERIC(12, 4);
COMMENT ON COLUMN etf_performance.nav_calc_date IS '淨值含息報酬的基準日（最後一筆淨值；當天正式淨值未公布時用預估淨值）';
COMMENT ON COLUMN etf_performance.nav_ret_1y IS '以正式淨值計算、現金配息還原的報酬（%）；nav_ret_* 規則同 ret_*，1 年以上年化，歷史不足為 NULL';

CREATE TABLE IF NOT EXISTS etf_snapshot
(
    stock_symbol       VARCHAR(24)                            NOT NULL,
    name               TEXT                                   NOT NULL,
    full_name          TEXT                                   NOT NULL DEFAULT '',
    market_id          INTEGER                                NOT NULL, -- 2 上市、4 上櫃
    strategy           TEXT                                   NOT NULL DEFAULT '',
    region             TEXT                                   NOT NULL DEFAULT '',
    themes             TEXT[]                                 NOT NULL DEFAULT '{}',
    dividend_frequency TEXT                                   NOT NULL DEFAULT '',
    issuer             TEXT                                   NOT NULL DEFAULT '',
    benchmark          TEXT                                   NOT NULL DEFAULT '',
    quote_date         DATE,
    closing_price      NUMERIC(18, 4),
    change             NUMERIC(18, 4),
    change_range       NUMERIC(12, 4),
    trading_volume     BIGINT, -- 張
    trade_value        NUMERIC(20, 2), -- 元
    nav_date           DATE,
    nav_time           TIMESTAMP WITH TIME ZONE,
    nav                NUMERIC(18, 4), -- 預估淨值
    premium_rate       NUMERIC(10, 4),
    premium_avg_20d    NUMERIC(10, 4), -- 近 20 筆折溢價平均
    outstanding_units  BIGINT,
    total_assets       NUMERIC(20, 2), -- 已發行單位 × 淨值（元）
    units_change       BIGINT,
    net_flow           NUMERIC(20, 2), -- 單位增減 × 淨值（元）
    dividend_12m       NUMERIC(18, 4), -- 近 12 個月現金配息合計
    dividend_count_12m INTEGER,
    yield_12m          NUMERIC(12, 4), -- 近 12 個月配息 / 收盤價（%）
    last_dividend      NUMERIC(18, 4),
    last_ex_date       DATE,
    next_dividend      NUMERIC(18, 4),
    next_ex_date       DATE,
    ret_1w             NUMERIC(12, 4),
    ret_1m             NUMERIC(12, 4),
    ret_3m             NUMERIC(12, 4),
    ret_6m             NUMERIC(12, 4),
    ret_ytd            NUMERIC(12, 4),
    ret_1y             NUMERIC(12, 4),
    year_low           NUMERIC(18, 4),
    year_high          NUMERIC(18, 4),
    holders            BIGINT, -- 集保受益人數
    holders_date       DATE,
    foreign_net        BIGINT, -- 外資買賣超（股）
    chip_date          DATE,
    updated_at         TIMESTAMP WITH TIME ZONE DEFAULT NOW() NOT NULL,
    CONSTRAINT etf_snapshot_pkey PRIMARY KEY (stock_symbol)
);

COMMENT ON TABLE etf_snapshot IS 'ETF 專區清單頁的預先彙整（每檔一列）；由 stock_crawler 在淨值、績效、籌碼入庫後重建';
