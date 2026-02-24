create table public.revenue_last_date
(
    security_code varchar(64)              default ''::character varying                   not null
        primary key,
    serial        bigint                   default 0                                       not null,
    created_time  timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    stock_symbol  varchar(64)              default ''::character varying                   not null
);

comment on column public.revenue_last_date.stock_symbol is '股票代碼';
create unique index revenue_last_date_stock_symbol_uidx on public.revenue_last_date (stock_symbol);