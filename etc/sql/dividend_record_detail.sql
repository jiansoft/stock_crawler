-- 持股股息發放記錄表
-- DROP TABLE IF EXISTS public.dividend_record_detail;
create table if not exists public.dividend_record_detail
(
    stock_ownership_details_serial bigint                   default 0                                       not null,
    year                           integer                  default 0                                       not null,
    cash                           numeric(18, 4)           default 0                                       not null,
    stock_money                    numeric(18, 4)           default 0                                       not null,
    stock                          numeric(18, 4)           default 0                                       not null,
    total                          numeric(18, 4)           default 0                                       not null,
    created_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    serial                         bigserial,
    primary key (stock_ownership_details_serial, year)
);

comment on column public.dividend_record_detail.year is '資料屬於那一年度';
comment on column public.dividend_record_detail.cash is '現金股利(元)';
comment on column public.dividend_record_detail.stock_money is '股票股利(元)';
comment on column public.dividend_record_detail.stock is '股票股利(股)';
comment on column public.dividend_record_detail.total is '合計股利(元)';

create unique index if not exists "dividend_record_detail-serial-idx"
    on public.dividend_record_detail (serial);

/*
某公司股價100元配現金0.7元、配股3.6元(以一張為例)
現金股利＝1張ｘ1000股x股利0.7元=700元
股票股利＝1張x1000股x股利0.36=360股
(股票股利須除以發行面額10元)
*/


