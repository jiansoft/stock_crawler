CREATE TABLE public.income_statement (
    stock_symbol                varchar(24) NOT NULL,
    fiscal_year                 integer NOT NULL,
    period_type                 varchar(12) NOT NULL,
    quarter                     varchar(4) NOT NULL,
    revenue                     bigint,
    gross_profit                bigint,
    selling_expenses            bigint,
    admin_expenses              bigint,
    rd_expenses                 bigint,
    operating_expenses          bigint,
    operating_profit            bigint,
    non_operating_income        bigint,
    profit_before_tax           bigint,
    net_income                  bigint,
    owner_parent_profit         bigint,
    revenue_per_share           numeric(12, 2),
    operating_profit_per_share  numeric(12, 2),
    profit_before_tax_per_share numeric(12, 2),
    eps                         numeric(12, 2),
    bps                         numeric(12, 2),
    source                      varchar(24) NOT NULL DEFAULT 'yahoo',
    fetched_at                  timestamptz NOT NULL DEFAULT now(),
    updated_at                  timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT income_statement_pkey
        PRIMARY KEY (stock_symbol, fiscal_year, period_type, quarter, source),
    CONSTRAINT income_statement_period_type_check
        CHECK (period_type IN ('single', 'cumulative', 'annual')),
    CONSTRAINT income_statement_quarter_check
        CHECK (
            (period_type = 'annual' AND quarter = 'A')
            OR
            (period_type IN ('single', 'cumulative') AND quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))
        )
);

COMMENT ON TABLE public.income_statement IS
    '股票損益表；金額統一為新台幣千元，每股數值為新台幣元，NULL 表示來源未提供資料';

COMMENT ON COLUMN public.income_statement.stock_symbol IS
    '股票代號，不含市場後綴，例如 8042';
COMMENT ON COLUMN public.income_statement.fiscal_year IS
    '財報所屬年度，例如 2026';
COMMENT ON COLUMN public.income_statement.period_type IS
    '資料口徑：single 單季、cumulative 年初至該季累計、annual 全年度；目前只寫入 single 與 annual，累計金額可由單季加總推得（累季 EPS 除外），cumulative 保留供日後使用';
COMMENT ON COLUMN public.income_statement.quarter IS
    '季別：單季或累季為 Q1 至 Q4；全年度為 A（與 financial_statement、dividend 一致）';
COMMENT ON COLUMN public.income_statement.revenue IS
    '營業收入，單位為新台幣千元';
COMMENT ON COLUMN public.income_statement.gross_profit IS
    '營業毛利，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.income_statement.selling_expenses IS
    '推銷費用，單位為新台幣千元；金融業來源多為 0';
COMMENT ON COLUMN public.income_statement.admin_expenses IS
    '管理費用，單位為新台幣千元；金融業來源多為 0';
COMMENT ON COLUMN public.income_statement.rd_expenses IS
    '研究發展費用，單位為新台幣千元；金融業來源多為 0';
COMMENT ON COLUMN public.income_statement.operating_expenses IS
    '營業費用，單位為新台幣千元';
COMMENT ON COLUMN public.income_statement.operating_profit IS
    '營業利益，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.income_statement.non_operating_income IS
    '營業外收入及支出，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.income_statement.profit_before_tax IS
    '稅前淨利，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.income_statement.net_income IS
    '本期稅後淨利（含非控制權益），單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.income_statement.owner_parent_profit IS
    '歸屬母公司業主淨利，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.income_statement.revenue_per_share IS
    '每股營收，單位為新台幣元';
COMMENT ON COLUMN public.income_statement.operating_profit_per_share IS
    '每股營業利益，單位為新台幣元，可為負數';
COMMENT ON COLUMN public.income_statement.profit_before_tax_per_share IS
    '每股稅前淨利，單位為新台幣元，可為負數';
COMMENT ON COLUMN public.income_statement.eps IS
    '每股盈餘，單位為新台幣元；年度與累季為來源官方值，不等於各單季相加';
COMMENT ON COLUMN public.income_statement.bps IS
    '每股淨值，單位為新台幣元；為期末時點數值，不可跨期相加';
COMMENT ON COLUMN public.income_statement.source IS
    '資料來源識別，例如 yahoo';
COMMENT ON COLUMN public.income_statement.fetched_at IS
    '最近一次成功取得來源資料的時間';
COMMENT ON COLUMN public.income_statement.updated_at IS
    '資料數值最近一次變更的時間；由寫入程式維護';

CREATE INDEX income_statement_history_idx
    ON public.income_statement
    (source, stock_symbol, period_type, fiscal_year DESC, quarter DESC);
