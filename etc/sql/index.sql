create table public.index
(
    serial         bigserial
        primary key,
    category       varchar(255)             default ''::character varying                   not null,
    date           date                     default CURRENT_DATE                            not null,
    trading_volume numeric(18, 4)           default 0                                       not null,
    transaction    numeric(18, 4)           default 0                                       not null,
    trade_value    numeric(18, 4)           default 0                                       not null,
    change         numeric(18, 4)           default 0                                       not null,
    index          numeric(18, 4)           default 0                                       not null,
    create_time    timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    update_time    timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

comment on column public.index.category is '分類';
comment on column public.index.date is '資料屬於那一天';
comment on column public.index.trading_volume is '成交股數';
comment on column public.index.transaction is '成交筆數';
comment on column public.index.trade_value is '成交金額';
comment on column public.index.change is '漲跌點數';
comment on column public.index.index is '指數';

create unique index "index-date_category-uidx"
    on public.index (date, category);

