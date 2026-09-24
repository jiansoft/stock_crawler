create table public.dividend_record_detail_more
(
    serial                         bigserial,
    stock_ownership_details_serial bigint                   default 0                                       not null,
    dividend_record_detail_serial  bigint                   default 0                                       not null,
    dividend_serial                bigint                   default 0                                       not null,
    cash                           numeric(18, 4)           default 0                                       not null,
    stock_money                    numeric(18, 4)           default 0                                       not null,
    stock                          numeric(18, 4)           default 0                                       not null,
    total                          numeric(18, 4)           default 0                                       not null,
    created_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    primary key (stock_ownership_details_serial, dividend_record_detail_serial, dividend_serial)
);

comment on column public.dividend_record_detail_more.stock_ownership_details_serial is '持股名細表的編號';
comment on column public.dividend_record_detail_more.dividend_record_detail_serial is '持股股息發放記錄表的編號(總計表)';
comment on column public.dividend_record_detail_more.dividend_serial is '股利發放明細表的編號';
comment on column public.dividend_record_detail_more.cash is '現金股利(元)';
comment on column public.dividend_record_detail_more.stock_money is '股票股利(元)';
comment on column public.dividend_record_detail_more.stock is '股票股利(股)';
comment on column public.dividend_record_detail_more.total is '合計股利(元)';
