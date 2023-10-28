create table public.stocks
(
    "SecurityCode"                varchar(24)              default ''::character varying                   not null,
    "Name"                        varchar(255)             default ''::character varying                   not null,
    "CreateTime"                  timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    "CategoryId"                  integer                  default 0                                       not null,
    "SuspendListing"              boolean                  default false                                   not null,
    last_one_eps                  numeric(18, 4)           default 0                                       not null,
    last_four_eps                 numeric(18, 4)           default 0                                       not null,
    net_asset_value_per_share     numeric(18, 4)           default 0                                       not null,
    stock_symbol                  varchar(24)              default ''::character varying                   not null
        primary key,
    stock_exchange_market_id      integer                  default 0                                       not null,
    stock_industry_id             integer                  default 0                                       not null,
    return_on_equity              numeric(18, 4)           default 0                                       not null,
    weight                        numeric(18, 4)           default 0                                       not null,
    issued_share                  bigint                   default 0                                       not null,
    qfii_shares_held              bigint                   default 0                                       not null,
    qfii_share_holding_percentage numeric(18, 4)           default 0                                       not null
);

comment on column public.stocks.last_one_eps is '近一季EPS';
comment on column public.stocks.last_four_eps is '近四季EPS';
comment on column public.stocks.net_asset_value_per_share is '每股淨值';
comment on column public.stocks.stock_exchange_market_id is '交易所的市場編號參考 stock_exchange_market';
comment on column public.stocks.stock_industry_id is '股票的產業分類編號 stock_industry';
comment on column public.stocks.return_on_equity is '股東權益報酬率';
comment on column public.stocks.weight is '權值佔比';
comment on column public.stocks.issued_share is '發行股數';
comment on column public.stocks.qfii_shares_held is '全體外資及陸資持有股數';
comment on column public.stocks.qfii_share_holding_percentage is '全體外資及陸資持股比率';

create index "stocks-stock_exchange_market_id-stock_industry_id-idx"
    on public.stocks (stock_exchange_market_id, stock_industry_id);

create index "stocks-stock_industry_id-idx"
    on public.stocks (stock_industry_id);


