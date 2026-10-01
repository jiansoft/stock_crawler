CREATE TABLE public.cash_flow_statement (
    stock_symbol        varchar(24) NOT NULL,
    fiscal_year         integer NOT NULL,
    period_type         varchar(12) NOT NULL,
    quarter             varchar(4) NOT NULL,
    depreciation        bigint,
    amortization        bigint,
    operating_cash_flow bigint,
    investing_cash_flow bigint,
    financing_cash_flow bigint,
    free_cash_flow      bigint,
    net_cash_flow       bigint,
    source              varchar(24) NOT NULL DEFAULT 'yahoo',
    fetched_at          timestamptz NOT NULL DEFAULT now(),
    updated_at          timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT cash_flow_statement_pkey
        PRIMARY KEY (stock_symbol, fiscal_year, period_type, quarter, source),
    CONSTRAINT cash_flow_statement_period_type_check
        CHECK (period_type IN ('single', 'cumulative', 'annual')),
    CONSTRAINT cash_flow_statement_quarter_check
        CHECK (
            (period_type = 'annual' AND quarter = 'A')
            OR
            (period_type IN ('single', 'cumulative') AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))
        )
);

COMMENT ON TABLE public.cash_flow_statement IS
    '股票現金流量表；金額統一為新台幣千元，NULL 表示來源未提供資料';

COMMENT ON COLUMN public.cash_flow_statement.stock_symbol IS
    '股票代號，不含市場後綴，例如 8042';
COMMENT ON COLUMN public.cash_flow_statement.fiscal_year IS
    '財報所屬年度，例如 2026';
COMMENT ON COLUMN public.cash_flow_statement.period_type IS
    '資料口徑：single 單季、cumulative 年初至該季累計、annual 全年度；目前只寫入 single 與 annual，累計可由單季加總推得，cumulative 保留供日後使用';
COMMENT ON COLUMN public.cash_flow_statement.quarter IS
    '季別：單季或累季為 Q1 至 Q4；全年度為 A（與 financial_statement、dividend 一致）';
COMMENT ON COLUMN public.cash_flow_statement.depreciation IS
    '折舊費用，單位為新台幣千元；可與營業利益、攤銷合計推算 EBITDA';
COMMENT ON COLUMN public.cash_flow_statement.amortization IS
    '攤銷費用，單位為新台幣千元；可與營業利益、折舊合計推算 EBITDA';
COMMENT ON COLUMN public.cash_flow_statement.operating_cash_flow IS
    '營業活動現金流量，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.cash_flow_statement.investing_cash_flow IS
    '投資活動現金流量，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.cash_flow_statement.financing_cash_flow IS
    '融資活動現金流量，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.cash_flow_statement.free_cash_flow IS
    '來源提供的自由現金流；Yahoo 定義為營業現金流加投資現金流，單位為新台幣千元';
COMMENT ON COLUMN public.cash_flow_statement.net_cash_flow IS
    '來源提供的現金及約當現金淨增減，單位為新台幣千元；不可直接以其他三項現金流相加取代';
COMMENT ON COLUMN public.cash_flow_statement.source IS
    '資料來源識別，例如 yahoo';
COMMENT ON COLUMN public.cash_flow_statement.fetched_at IS
    '最近一次成功取得來源資料的時間';
COMMENT ON COLUMN public.cash_flow_statement.updated_at IS
    '資料數值最近一次變更的時間；由寫入程式維護';

CREATE INDEX cash_flow_statement_history_idx
    ON public.cash_flow_statement
    (source, stock_symbol, period_type, fiscal_year DESC, quarter DESC);
