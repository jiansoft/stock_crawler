create table if not exists public.chip_daily
(
    "date"          date                                                   not null,
    stock_symbol    varchar(24)              default ''::character varying not null,
    foreign_net     bigint,
    trust_net       bigint,
    dealer_net      bigint,
    margin_previous bigint,
    margin_balance  bigint,
    short_previous  bigint,
    short_balance   bigint,
    created_time    timestamp with time zone default now()                 not null,
    updated_time    timestamp with time zone default now()                 not null,
    primary key ("date", stock_symbol)
);

comment on table public.chip_daily is '每日籌碼：三大法人買賣超（上市 T86、上櫃 3itrade_hedge_result）與融資融券餘額（上市 MI_MARGN、上櫃 margin_bal_result），全市場每個交易日一筆';

comment on column public.chip_daily."date" is '交易日';
comment on column public.chip_daily.stock_symbol is '股票代號';
comment on column public.chip_daily.foreign_net is '外資及陸資（含外資自營商）買賣超股數；來源沒有此股時為 NULL';
comment on column public.chip_daily.trust_net is '投信買賣超股數';
comment on column public.chip_daily.dealer_net is '自營商（自行買賣與避險合計）買賣超股數';
comment on column public.chip_daily.margin_previous is '前日融資餘額（張）；不能信用交易的股票為 NULL';
comment on column public.chip_daily.margin_balance is '今日融資餘額（張）';
comment on column public.chip_daily.short_previous is '前日融券餘額（張）';
comment on column public.chip_daily.short_balance is '今日融券餘額（張）';

-- 依股票查歷史（近 N 日買賣超、連續天數）。
create index if not exists "chip_daily-stock_symbol-date-idx"
    on public.chip_daily (stock_symbol, "date" desc);
