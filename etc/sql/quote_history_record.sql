create table public.quote_history_record
(
    security_code                         varchar(24)    default ''::character varying not null
        primary key,
    maximum_price                         numeric(18, 4) default 0                     not null,
    maximum_price_date_on                 date           default '1970-01-01'::date    not null,
    minimum_price                         numeric(18, 4) default 0                     not null,
    minimum_price_date_on                 date           default '1970-01-01'::date    not null,
    "maximum_price-to-book_ratio"         numeric(18, 4) default 0                     not null,
    "maximum_price-to-book_ratio_date_on" date           default '1970-01-01'::date    not null,
    "minimum_price-to-book_ratio"         numeric(18, 4) default 0                     not null,
    "minimum_price-to-book_ratio_date_on" date           default '1970-01-01'::date    not null
);

comment on column public.quote_history_record.maximum_price is '歷史最高價';
comment on column public.quote_history_record.maximum_price_date_on is '歷史最高價出現在哪一天(系統內收集的數據)';
comment on column public.quote_history_record.minimum_price is '歷史最低價';
comment on column public.quote_history_record.minimum_price_date_on is '歷史最低價出現在哪一天(系統內收集的數據)';
comment on column public.quote_history_record."maximum_price-to-book_ratio" is '歷史最高股價淨值比';
comment on column public.quote_history_record."maximum_price-to-book_ratio_date_on" is '歷史最高股價淨值比出現在哪一天(系統內收集的數據)';
comment on column public.quote_history_record."minimum_price-to-book_ratio" is '歷史最高股價淨值比';
comment on column public.quote_history_record."minimum_price-to-book_ratio_date_on" is '歷史最低股價淨值比出現在哪一天(系統內收集的數據)';


