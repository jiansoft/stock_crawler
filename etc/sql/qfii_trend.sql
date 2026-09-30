create table if not exists public.qfii_trend
(
    stock_symbol             varchar(24)              default ''::character varying not null
        primary key,
    "date"                   date                                                   not null,
    share_holding_percentage numeric(8, 4)            default 0                     not null,
    change_5d                numeric(8, 4),
    change_20d               numeric(8, 4),
    streak                   integer                  default 0                     not null,
    updated_time             timestamp with time zone default now()                 not null
);

comment on table public.qfii_trend is '外資持股趨勢：每個交易日由 qfii_history 整批重算，只保留最新交易日仍有資料的股票';

comment on column public.qfii_trend.stock_symbol is '股票代號';
comment on column public.qfii_trend."date" is '趨勢基準日（qfii_history 的最新交易日）';
comment on column public.qfii_trend.share_holding_percentage is '基準日的外資及陸資持股比率（%）';
comment on column public.qfii_trend.change_5d is '近 5 個交易日持股比率變化（百分點）；歷史不足 6 筆時為 NULL';
comment on column public.qfii_trend.change_20d is '近 20 個交易日持股比率變化（百分點）；歷史不足 21 筆時為 NULL';
comment on column public.qfii_trend.streak is '以持有股數判斷的連續天數：正數為連續增持、負數為連續減持、0 為基準日持平';
