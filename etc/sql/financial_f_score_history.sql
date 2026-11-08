CREATE TABLE IF NOT EXISTS public.financial_f_score_history (
    stock_symbol      varchar(24) NOT NULL,
    fiscal_year       integer     NOT NULL,
    f_score           smallint    NOT NULL,
    f_score_evaluated smallint    NOT NULL,
    updated_at        timestamptz NOT NULL DEFAULT now(),

    CONSTRAINT financial_f_score_history_pkey PRIMARY KEY (stock_symbol, fiscal_year),
    CONSTRAINT financial_f_score_history_score_check
        CHECK (f_score BETWEEN 0 AND 9 AND f_score_evaluated BETWEEN 0 AND 9)
);

COMMENT ON TABLE public.financial_f_score_history IS
    '每檔股票每個年度的 Piotroski F 分數（與前一年度比較）；條件選股回測依換股日當時已公布的年報取用（隔年 4/1 起），每日與 financial_health 一起整批重算';
COMMENT ON COLUMN public.financial_f_score_history.stock_symbol IS '股票代號，不含市場後綴';
COMMENT ON COLUMN public.financial_f_score_history.fiscal_year IS '評分年度（有年度損益表的年度）';
COMMENT ON COLUMN public.financial_f_score_history.f_score IS 'Piotroski F 分數（0–9），通過的指標數';
COMMENT ON COLUMN public.financial_f_score_history.f_score_evaluated IS '可判定的指標數（0–9）';
COMMENT ON COLUMN public.financial_f_score_history.updated_at IS '最後重算時間';
