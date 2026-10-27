create table if not exists public.holder_distribution_level
(
    "date"       date                                                   not null,
    stock_symbol varchar(24)              default ''::character varying not null,
    level        smallint                                               not null,
    holders      bigint                   default 0                     not null,
    shares       bigint                   default 0                     not null,
    percent      numeric(8, 4)            default 0                     not null,
    created_time timestamp with time zone default now()                 not null,
    updated_time timestamp with time zone default now()                 not null,
    primary key ("date", stock_symbol, level)
);

comment on table public.holder_distribution_level is '集保戶股權分散表全部持股分級：每週六公布上週五的資料（集保開放資料只有最新一週，無法回補）';

comment on column public.holder_distribution_level."date" is '資料日期（集保的週資料日）';
comment on column public.holder_distribution_level.stock_symbol is '證券代號';
comment on column public.holder_distribution_level.level is '持股分級：1～15 依持股數由小到大（1 為 1～999 股、12 為 400,001～600,000 股、13 為 600,001～800,000 股、14 為 800,001～1,000,000 股、15 為 1,000,001 股以上），16 差異數調整，17 合計';
comment on column public.holder_distribution_level.holders is '人數';
comment on column public.holder_distribution_level.shares is '股數';
comment on column public.holder_distribution_level.percent is '占集保庫存數比例（%）';

create index if not exists "holder_distribution_level-stock_symbol-date-idx"
    on public.holder_distribution_level (stock_symbol, "date" desc);
