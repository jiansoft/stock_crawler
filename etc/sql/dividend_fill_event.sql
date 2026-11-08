-- dividend_fill_event：每檔、每個有現金股利的除息日一列，記錄除息前收盤與填息結果（衍生表）。
-- 每天 05:45 由 dividend 與 DailyQuotes 整批重算；條件選股與回測的殖利率、填息率、連續配息年數條件
-- 以這張表在換股日當天「已經知道」的事件計算，避免用到未來資料。

CREATE TABLE IF NOT EXISTS public.dividend_fill_event (
    stock_symbol varchar(24) NOT NULL,
    ex_date date NOT NULL,
    cash_dividend numeric(18, 4) NOT NULL,
    stock_dividend numeric(18, 4) NOT NULL DEFAULT 0,
    pre_ex_close numeric(18, 4),
    days_to_fill smallint,
    fill_date date,
    mature_date date,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT dividend_fill_event_pkey PRIMARY KEY (stock_symbol, ex_date),
    CONSTRAINT dividend_fill_event_cash_check CHECK (cash_dividend > 0)
);

COMMENT ON TABLE public.dividend_fill_event IS '每次現金除息的填息結果（每日 05:45 由 dividend、DailyQuotes 重算）';
COMMENT ON COLUMN public.dividend_fill_event.ex_date IS '除息日（同日多列時取各期列，不取全年彙總列）';
COMMENT ON COLUMN public.dividend_fill_event.cash_dividend IS '現金股利（元）';
COMMENT ON COLUMN public.dividend_fill_event.stock_dividend IS '同日除權的股票股利（元），用來調整填息目標價';
COMMENT ON COLUMN public.dividend_fill_event.pre_ex_close IS '除息前 30 天內最後一筆收盤；沒有日線（2012 年以前）為 NULL';
COMMENT ON COLUMN public.dividend_fill_event.days_to_fill IS '除息日起第幾個交易日收盤回到填息目標價（觀察 120 個交易日內），未填息為 NULL';
COMMENT ON COLUMN public.dividend_fill_event.fill_date IS '填息當天；未填息為 NULL';
COMMENT ON COLUMN public.dividend_fill_event.mature_date IS '除息日起第 120 個交易日（觀察期滿）；日線不足 120 筆為 NULL';
