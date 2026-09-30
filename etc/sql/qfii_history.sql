create table if not exists public.qfii_history
(
    stock_symbol             varchar(24)              default ''::character varying not null,
    "date"                   date                                                   not null,
    issued_share             bigint                   default 0                     not null,
    shares_held              bigint                   default 0                     not null,
    share_holding_percentage numeric(8, 4)            default 0                     not null,
    created_time             timestamp with time zone default now()                 not null,
    updated_time             timestamp with time zone default now()                 not null,
    primary key (stock_symbol, "date")
);

comment on table public.qfii_history is '每日外資及陸資持股快照：上市取自證交所 MI_QFIIS、上櫃取自櫃買 QFII API，每個交易日一筆';

comment on column public.qfii_history.stock_symbol is '股票代號';
comment on column public.qfii_history."date" is '資料日期（交易日）';
comment on column public.qfii_history.issued_share is '發行股數';
comment on column public.qfii_history.shares_held is '外資及陸資持有股數';
comment on column public.qfii_history.share_holding_percentage is '外資及陸資持股比率（%）';

-- 重算趨勢時要找「最新交易日」並取各股最近數十筆，依日期查詢的情境靠這個索引；
-- 依股票查歷史則走主鍵 (stock_symbol, date)。
create index if not exists "qfii_history-date-idx"
    on public.qfii_history ("date");
