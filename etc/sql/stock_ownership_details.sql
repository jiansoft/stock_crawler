create table public.stock_ownership_details
(
    serial                         bigserial
        primary key,
    member_id                      bigint                   default 0                                       not null,
    security_code                  varchar(24)              default ''::character varying                   not null,
    share_quantity                 bigint                   default 0                                       not null,
    holding_cost                   numeric(18, 4)           default 0                                       not null,
    share_price_average            numeric(18, 4)           default 0                                       not null,
    current_cost_per_share         numeric(18, 4)           default 0                                       not null,
    is_sold                        boolean                  default false,
    cumulate_dividends_cash        numeric(18, 4)           default 0,
    cumulate_dividends_stock       numeric(18, 4)           default 0,
    cumulate_dividends_stock_money numeric(18, 4)           default 0,
    cumulate_dividends_total       numeric(18, 4)           default 0,
    created_time                   timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    date                           date                     default CURRENT_DATE                            not null
);

comment on column public.stock_ownership_details.member_id is '會員編號 Member.Id';
comment on column public.stock_ownership_details.security_code is '股票代碼';
comment on column public.stock_ownership_details.share_quantity is '持有股數';
comment on column public.stock_ownership_details.holding_cost is '持有成本';
comment on column public.stock_ownership_details.share_price_average is '每股成本';
comment on column public.stock_ownership_details.current_cost_per_share is '目前每股成本';
comment on column public.stock_ownership_details.is_sold is '是否賣出';
comment on column public.stock_ownership_details.cumulate_dividends_cash is '累積現金股利(元)';
comment on column public.stock_ownership_details.cumulate_dividends_stock is '累積股票股利(股)';
comment on column public.stock_ownership_details.cumulate_dividends_stock_money is '累積股票股利(元)';
comment on column public.stock_ownership_details.cumulate_dividends_total is '總計累積股利(元)';
comment on column public.stock_ownership_details.date is '交易日期';

create index "stock_ownership_details-security_code_idx"
    on public.stock_ownership_details (security_code);

alter table stock_ownership_details  add current_cost_per_share numeric(18, 4) default 0 not null;
comment on column public.stock_ownership_details.current_cost_per_share is '目前每股成本';