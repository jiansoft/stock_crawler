CREATE TABLE public.financial_health (
    stock_symbol         varchar(24) NOT NULL,
    f_score_year         integer,
    f_score              smallint,
    f_score_evaluated    smallint NOT NULL DEFAULT 0,
    quarter_year         integer,
    quarter              smallint,
    gross_margin_percent numeric(14, 4),
    debt_ratio_percent   numeric(14, 4),
    free_cash_flow_ttm   bigint,
    updated_at           timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT financial_health_pkey PRIMARY KEY (stock_symbol),
    CONSTRAINT financial_health_f_score_check
        CHECK (f_score BETWEEN 0 AND 9 AND f_score_evaluated BETWEEN 0 AND 9),
    CONSTRAINT financial_health_quarter_check CHECK (quarter BETWEEN 1 AND 4)
);

COMMENT ON TABLE public.financial_health IS
    '每檔股票的財務體質：年度 Piotroski F 分數與最新一季的選股指標；每日由三大財務報表整批重算（app::calculation::financial_health）';

COMMENT ON COLUMN public.financial_health.stock_symbol IS
    '股票代號，不含市場後綴';
COMMENT ON COLUMN public.financial_health.f_score_year IS
    'F 分數的評分年度：最新一個有年度損益表的年度，與前一年度比較；NULL 表示沒有年度損益表';
COMMENT ON COLUMN public.financial_health.f_score IS
    'Piotroski F 分數（0–9），通過的指標數；NULL 表示沒有評分年度';
COMMENT ON COLUMN public.financial_health.f_score_evaluated IS
    '可判定的指標數（0–9）；資料不足或不適用（例如金融業沒有流動負債分類）的指標不計入';
COMMENT ON COLUMN public.financial_health.quarter_year IS
    '季報指標的期間年度：最新一個有單季損益表的季度';
COMMENT ON COLUMN public.financial_health.quarter IS
    '季報指標的期間季別，1 至 4';
COMMENT ON COLUMN public.financial_health.gross_margin_percent IS
    '該季毛利率（%）＝營業毛利 ÷ 營業收入 × 100；營收為 0 或缺值時為 NULL';
COMMENT ON COLUMN public.financial_health.debt_ratio_percent IS
    '該季季末負債比（%）＝負債總額 ÷ 資產總額 × 100';
COMMENT ON COLUMN public.financial_health.free_cash_flow_ttm IS
    '截至該季的近四季單季自由現金流合計，單位為新台幣千元；四季缺任一季為 NULL';
COMMENT ON COLUMN public.financial_health.updated_at IS
    '這次整批重算的時間';
