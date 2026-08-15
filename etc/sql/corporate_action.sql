create table if not exists public.corporate_action
(
    stock_symbol   varchar(24)              default ''::character varying not null,
    effective_date date                                                   not null,
    action_type    varchar(24)              default 'split'               not null,
    share_ratio    numeric(18, 8)                                         not null,
    note           varchar(255)             default ''::character varying not null,
    created_time   timestamp with time zone default now()                 not null,
    updated_time   timestamp with time zone default now()                 not null,
    primary key (stock_symbol, effective_date)
);

comment on table public.corporate_action is '公司行動（股票分割、反向分割、減資）：報價存的是原始成交價，這些事件造成的價格跳動必須另行調整';

comment on column public.corporate_action.stock_symbol is '股票代號';
comment on column public.corporate_action.effective_date is '生效日：換發後恢復交易的第一個交易日（該日收盤價已是調整後價格）';
comment on column public.corporate_action.action_type is '事件類型：split（分割）/ reverse_split（反向分割）/ capital_reduction（減資）';
comment on column public.corporate_action.share_ratio is '股數變動比例：持有 1 股在事件後變成幾股。1 股分割成 4 股為 4；減資三成為 0.7';
comment on column public.corporate_action.note is '備註，例如「1:4 分割」「減資彌補虧損」';

-- 計算 CAGR 時以「生效日 > 期初日」為條件一次撈齊區間內的所有事件，
-- 因此以日期為前導欄位；資料量小（一年數十筆），單一索引已足夠。
create index if not exists "corporate_action-effective_date-idx"
    on public.corporate_action (effective_date);
