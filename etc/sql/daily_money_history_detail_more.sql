create table public.daily_money_history_detail_more
(
    serial                     bigserial
        primary key,
    member_id                  bigint                   default 0                                       not null,
    date                       date                     default CURRENT_DATE                            not null,
    transaction_date           date                     default CURRENT_DATE                            not null,
    security_code              varchar(24)              default ''::character varying                   not null,
    closing_price              numeric(18, 4)           default 0                                       not null,
    number_of_shares_held      bigint                   default 0                                       not null,
    unit_price_per_share       numeric(18, 4)           default 0                                       not null,
    cost                       numeric(18, 4)           default 0                                       not null,
    market_value               numeric(18, 4)           default 0                                       not null,
    profit_and_loss            numeric(18, 4)           default 0                                       not null,
    profit_and_loss_percentage numeric(18, 4)           default 0                                       not null,
    created_time               timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time               timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

comment on column public.daily_money_history_detail_more.member_id is '資料屬於那一個會員';
comment on column public.daily_money_history_detail_more.date is '記錄所屬日期';
comment on column public.daily_money_history_detail_more.transaction_date is '成交日期';
comment on column public.daily_money_history_detail_more.closing_price is '當日收盤價格(參考價)';
comment on column public.daily_money_history_detail_more.number_of_shares_held is '股數';
comment on column public.daily_money_history_detail_more.unit_price_per_share is '成交價(每股單價)';
comment on column public.daily_money_history_detail_more.cost is '投資成本(負數 -shares * unit_price_per_share)';
comment on column public.daily_money_history_detail_more.market_value is '市值 (shares * closing_price)';
comment on column public.daily_money_history_detail_more.profit_and_loss is '預估損益';
comment on column public.daily_money_history_detail_more.profit_and_loss_percentage is '預估獲利率';


create index "daily_money_history_detail_more-date-member_id"
    on public.daily_money_history_detail_more (date, member_id);

