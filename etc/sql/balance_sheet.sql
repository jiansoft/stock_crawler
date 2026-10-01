CREATE TABLE public.balance_sheet (
    stock_symbol                             varchar(24) NOT NULL,
    fiscal_year                              integer NOT NULL,
    period_type                              varchar(12) NOT NULL,
    quarter                                  varchar(4) NOT NULL,
    cash_and_equivalents                     bigint,
    short_term_investments                   bigint,
    accounts_receivable                      bigint,
    inventory                                bigint,
    other_current_assets                     bigint,
    current_assets                           bigint,
    equity_and_other_investments             bigint,
    property_plant_equipment                 bigint,
    right_of_use_asset                       bigint,
    non_current_assets                       bigint,
    total_assets                             bigint,
    long_term_investment                     bigint,
    short_term_investment                    bigint,
    other_assets                             bigint,
    short_term_debt                          bigint,
    short_term_bills_payable                 bigint,
    accounts_payable                         bigint,
    current_portion_of_long_term_liabilities bigint,
    current_liabilities                      bigint,
    long_term_liabilities                    bigint,
    bonds_payable                            bigint,
    other_liabilities                        bigint,
    non_current_liabilities                  bigint,
    total_liabilities                        bigint,
    share_capital                            bigint,
    retained_earnings                        bigint,
    equity                                   bigint,
    book_value_per_share                     numeric(12, 2),
    source                                   varchar(24) NOT NULL DEFAULT 'yahoo',
    fetched_at                               timestamptz NOT NULL DEFAULT now(),
    updated_at                               timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT balance_sheet_pkey
        PRIMARY KEY (stock_symbol, fiscal_year, period_type, quarter, source),
    CONSTRAINT balance_sheet_period_type_check
        CHECK (period_type = 'single'),
    CONSTRAINT balance_sheet_quarter_check
        CHECK (quarter IN ('Q1', 'Q2', 'Q3', 'Q4'))
);

COMMENT ON TABLE public.balance_sheet IS
    '股票資產負債表（季末時點數值）；金額統一為新台幣千元，每股數值為新台幣元，NULL 表示來源未提供資料';

COMMENT ON COLUMN public.balance_sheet.stock_symbol IS
    '股票代號，不含市場後綴，例如 8042';
COMMENT ON COLUMN public.balance_sheet.fiscal_year IS
    '財報所屬年度，例如 2026';
COMMENT ON COLUMN public.balance_sheet.period_type IS
    '資料口徑：僅 single（季末）；資產負債表為時點數值，全年度即為第 4 季，來源的年度資料亦混有逐日雜訊而不採用';
COMMENT ON COLUMN public.balance_sheet.quarter IS
    '季別：Q1 至 Q4（與 financial_statement、dividend 一致）';
COMMENT ON COLUMN public.balance_sheet.cash_and_equivalents IS
    '現金及約當現金，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.short_term_investments IS
    '短期投資，Yahoo shortTermInvestments，單位為新台幣千元；與 short_term_investment 為來源的兩個不同欄位，數值不同，不可合併';
COMMENT ON COLUMN public.balance_sheet.accounts_receivable IS
    '應收帳款及票據，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.inventory IS
    '存貨，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.other_current_assets IS
    '其他流動資產，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.current_assets IS
    '流動資產，單位為新台幣千元；金融業來源為 0';
COMMENT ON COLUMN public.balance_sheet.equity_and_other_investments IS
    '權益法及其他投資，Yahoo equityAndOtherInvestments，單位為新台幣千元；語意依欄位名推斷';
COMMENT ON COLUMN public.balance_sheet.property_plant_equipment IS
    '不動產、廠房及設備，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.right_of_use_asset IS
    '使用權資產，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.non_current_assets IS
    '非流動資產，單位為新台幣千元；金融業來源為 0';
COMMENT ON COLUMN public.balance_sheet.total_assets IS
    '資產總額，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.long_term_investment IS
    '長期投資，Yahoo longTermInvestment，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.short_term_investment IS
    '短期投資，Yahoo shortTermInvestment，單位為新台幣千元；與 short_term_investments 為來源的兩個不同欄位，數值不同，不可合併';
COMMENT ON COLUMN public.balance_sheet.other_assets IS
    '其他資產，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.short_term_debt IS
    '短期借款，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.short_term_bills_payable IS
    '應付短期票券，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.accounts_payable IS
    '應付帳款及票據，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.current_portion_of_long_term_liabilities IS
    '一年內到期長期負債，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.current_liabilities IS
    '流動負債，單位為新台幣千元；金融業來源為 0';
COMMENT ON COLUMN public.balance_sheet.long_term_liabilities IS
    '長期負債，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.bonds_payable IS
    '應付公司債，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.other_liabilities IS
    '其他負債，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.non_current_liabilities IS
    '非流動負債，單位為新台幣千元；金融業來源為 0';
COMMENT ON COLUMN public.balance_sheet.total_liabilities IS
    '負債總額，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.share_capital IS
    '股本，單位為新台幣千元';
COMMENT ON COLUMN public.balance_sheet.retained_earnings IS
    '保留盈餘，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.balance_sheet.equity IS
    '權益總額，單位為新台幣千元，可為負數';
COMMENT ON COLUMN public.balance_sheet.book_value_per_share IS
    '每股淨值，Yahoo netWorth，單位為新台幣元，可為負數';
COMMENT ON COLUMN public.balance_sheet.source IS
    '資料來源識別，例如 yahoo';
COMMENT ON COLUMN public.balance_sheet.fetched_at IS
    '最近一次成功取得來源資料的時間';
COMMENT ON COLUMN public.balance_sheet.updated_at IS
    '資料數值最近一次變更的時間；由寫入程式維護';

CREATE INDEX balance_sheet_history_idx
    ON public.balance_sheet
    (source, stock_symbol, period_type, fiscal_year DESC, quarter DESC);
