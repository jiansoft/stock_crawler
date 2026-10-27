create table if not exists public.holder_distribution
(
    "date"        date                                                   not null,
    stock_symbol  varchar(24)              default ''::character varying not null,
    major_holders bigint                   default 0                     not null,
    major_percent numeric(8, 4)            default 0                     not null,
    total_holders bigint                   default 0                     not null,
    holders_400   bigint,
    shares_400    bigint,
    percent_400   numeric(8, 4),
    created_time  timestamp with time zone default now()                 not null,
    updated_time  timestamp with time zone default now()                 not null,
    primary key ("date", stock_symbol)
);

comment on table public.holder_distribution is '集保戶股權分散表週摘要：每週六公布上週五的資料（集保開放資料只有最新一週；2026-10-02 以前的週由神秘金字塔 norway.twsthr.info 一次性回補）';

comment on column public.holder_distribution."date" is '資料日期（集保的週資料日）';
comment on column public.holder_distribution.stock_symbol is '證券代號';
comment on column public.holder_distribution.major_holders is '千張大戶（1,000,001 股以上）人數';
comment on column public.holder_distribution.major_percent is '千張大戶持股佔集保庫存比例（%）';
comment on column public.holder_distribution.total_holders is '集保股東總人數';
comment on column public.holder_distribution.holders_400 is '400 張以上（400,001 股以上，第 12～15 級）人數；NULL 為沒有分級資料';
comment on column public.holder_distribution.shares_400 is '400 張以上股數（集保週為精確股數；回補週為網站的張數 × 1000）；NULL 為沒有分級資料';
comment on column public.holder_distribution.percent_400 is '400 張以上持股佔集保庫存比例（%，由股數除以合計股數重算）；NULL 為沒有分級資料';

create index if not exists "holder_distribution-stock_symbol-date-idx"
    on public.holder_distribution (stock_symbol, "date" desc);
