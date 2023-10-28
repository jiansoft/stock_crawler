create table public.last_daily_quotes
(
    date                          date                     default CURRENT_DATE                            not null,
    security_code                 varchar(24)              default ''::character varying                   not null
        primary key,
    trading_volume                numeric(18, 4)           default 0                                       not null,
    transaction                   numeric(18, 4)           default 0                                       not null,
    trade_value                   numeric(18, 4)           default 0                                       not null,
    opening_price                 numeric(18, 4)           default 0                                       not null,
    highest_price                 numeric(18, 4)           default 0                                       not null,
    lowest_price                  numeric(18, 4)           default 0                                       not null,
    closing_price                 numeric(18, 4)           default 0                                       not null,
    change_range                  numeric(18, 4)           default 0                                       not null,
    change                        numeric(18, 4)           default 0                                       not null,
    last_best_bid_price           numeric(18, 4)           default 0                                       not null,
    last_best_bid_volume          numeric(18, 4)           default 0                                       not null,
    last_best_ask_price           numeric(18, 4)           default 0                                       not null,
    last_best_ask_volume          numeric(18, 4)           default 0                                       not null,
    price_earning_ratio           numeric(18, 4)           default 0                                       not null,
    moving_average_5              numeric(18, 4)           default 0                                       not null,
    moving_average_10             numeric(18, 4)           default 0                                       not null,
    moving_average_20             numeric(18, 4)           default 0                                       not null,
    moving_average_60             numeric(18, 4)           default 0                                       not null,
    moving_average_120            numeric(18, 4)           default 0                                       not null,
    moving_average_240            numeric(18, 4)           default 0                                       not null,
    maximum_price_in_year         numeric(18, 4)           default 0                                       not null,
    minimum_price_in_year         numeric(18, 4)           default 0                                       not null,
    average_price_in_year         numeric(18, 4)           default 0                                       not null,
    maximum_price_in_year_date_on date                     default '1970-01-01'::date                      not null,
    minimum_price_in_year_date_on date                     default '1970-01-01'::date                      not null,
    "price-to-book_ratio"         numeric(18, 4)           default 0                                       not null,
    record_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time                  timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

comment on column public.last_daily_quotes.date is '資料屬於那一天';
comment on column public.last_daily_quotes.security_code is '股票代碼';
comment on column public.last_daily_quotes.trading_volume is '成交股數';
comment on column public.last_daily_quotes.transaction is '成交筆數';
comment on column public.last_daily_quotes.trade_value is '成交金額';
comment on column public.last_daily_quotes.opening_price is '開盤價';
comment on column public.last_daily_quotes.highest_price is '最高價';
comment on column public.last_daily_quotes.lowest_price is '最低價';
comment on column public.last_daily_quotes.closing_price is '收盤價';
comment on column public.last_daily_quotes.change_range is '漲幅';
comment on column public.last_daily_quotes.change is '漲跌價差';
comment on column public.last_daily_quotes.last_best_bid_price is '最後揭示買價';
comment on column public.last_daily_quotes.last_best_bid_volume is '最後揭示買量';
comment on column public.last_daily_quotes.last_best_ask_price is '最後揭示賣價';
comment on column public.last_daily_quotes.last_best_ask_volume is '最後揭示賣量';
comment on column public.last_daily_quotes.price_earning_ratio is '本益比';
comment on column public.last_daily_quotes.moving_average_5 is '5日週線';
comment on column public.last_daily_quotes.moving_average_10 is '10日雙週線';
comment on column public.last_daily_quotes.moving_average_20 is '20日月線';
comment on column public.last_daily_quotes.moving_average_60 is '60日季線';
comment on column public.last_daily_quotes.moving_average_120 is '120日半年線';
comment on column public.last_daily_quotes.moving_average_240 is '240日年線';
comment on column public.last_daily_quotes.maximum_price_in_year is '一年內最高價(收盤日為起點)';
comment on column public.last_daily_quotes.minimum_price_in_year is '一年內最低價(收盤日為起點)';
comment on column public.last_daily_quotes.average_price_in_year is '一年內平均價(收盤日為起點)';
comment on column public.last_daily_quotes.maximum_price_in_year_date_on is '一年內最高價在哪一天(收盤日為起點)';
comment on column public.last_daily_quotes.minimum_price_in_year_date_on is '一年內最低價在哪一天(收盤日為起點)';
comment on column public.last_daily_quotes."price-to-book_ratio" is '股價淨值比';
comment on column public.last_daily_quotes.record_time is '資料日期';

