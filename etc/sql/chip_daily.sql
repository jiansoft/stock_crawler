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
    foreign_streak  smallint,
    trust_streak    smallint,
    dealer_streak   smallint,
    total_streak    smallint,
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
comment on column public.chip_daily.foreign_streak is '外資截至當日的連續買賣超天數：正數連續買超、負數連續賣超、0 為當日淨額 0 或沒有資料；只數這檔有籌碼列的日子，NULL 為尚未計算';
comment on column public.chip_daily.trust_streak is '投信連續買賣超天數（規則同 foreign_streak）';
comment on column public.chip_daily.dealer_streak is '自營商連續買賣超天數（規則同 foreign_streak）';
comment on column public.chip_daily.total_streak is '三大法人合計連續買賣超天數（規則同 foreign_streak，缺值以 0 相加）';

-- 依股票查歷史（近 N 日買賣超、連續天數）。
create index if not exists "chip_daily-stock_symbol-date-idx"
    on public.chip_daily (stock_symbol, "date" desc);

-- 預設 autovacuum 門檻是表的 20%（數十萬列），每天的更新／刪除與新增永遠達不到，死資料與
-- visibility map 都不更新、統計值逐漸過時（2026-11-04 M43 盤點）。改成每 2 萬列變動就 vacuum／analyze。
-- 正式庫 2026-11-05 已套用（M44）。
alter table public.chip_daily set (
    autovacuum_vacuum_insert_threshold = 20000,
    autovacuum_vacuum_insert_scale_factor = 0,
    autovacuum_vacuum_threshold = 20000,
    autovacuum_vacuum_scale_factor = 0,
    autovacuum_analyze_threshold = 20000,
    autovacuum_analyze_scale_factor = 0
);
