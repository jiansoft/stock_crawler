create table if not exists public.broker_flow
(
    "date"       date                                                   not null,
    stock_symbol varchar(24)              default ''::character varying not null,
    buy_total    bigint                   default 0                     not null,
    sell_total   bigint                   default 0                     not null,
    main_share   numeric(10, 4)           default 0                     not null,
    buyers       jsonb                    default '[]'::jsonb           not null,
    sellers      jsonb                    default '[]'::jsonb           not null,
    created_time timestamp with time zone default now()                 not null,
    updated_time timestamp with time zone default now()                 not null,
    primary key ("date", stock_symbol)
);

comment on table public.broker_flow is '主力進出（券商分點買賣超前幾名）：富邦 zco 頁，只抓持股（頁面只有最近一個交易日，無法回補）';

comment on column public.broker_flow."date" is '資料日期（頁面的最後更新日）';
comment on column public.broker_flow.stock_symbol is '股票代號';
comment on column public.broker_flow.buy_total is '頁面「合計買超張數」';
comment on column public.broker_flow.sell_total is '頁面「合計賣超張數」';
comment on column public.broker_flow.main_share is '主力買賣超佔成交量比重（%），正值為買超';
comment on column public.broker_flow.buyers is '買超前幾名 [{name, buy, sell, net, share}]，張數；share 為佔成交比重（%）';
comment on column public.broker_flow.sellers is '賣超前幾名，格式同 buyers';
