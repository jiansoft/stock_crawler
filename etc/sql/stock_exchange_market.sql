create table public.stock_exchange_market
(
    stock_exchange_market_id integer     default 0                     not null
        primary key,
    stock_exchange_id        integer     default 0                     not null,
    code                     varchar(24) default ''::character varying not null,
    name                     varchar(24) default ''::character varying not null
);

comment on column public.stock_exchange_market.stock_exchange_market_id is '交易所的市場編號';
comment on column public.stock_exchange_market.stock_exchange_id is '交易所的編號參考 stock_exchange';
comment on column public.stock_exchange_market.code is '交易所的市場代碼 TAI:上市 TWO:上櫃 TWE:興櫃';
comment on column public.stock_exchange_market.name is '市場名稱';

insert into stock_exchange_market (stock_exchange_market_id, code, name, stock_exchange_id)
values (2, 'TAI', '上市', 1),
       (4, 'TWO', '上櫃', 2),
       (5, 'TWE', '興櫃', 2);