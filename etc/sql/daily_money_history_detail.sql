create table if not exists public.daily_money_history_detail
(
    serial                                  bigserial,
    household_id                            bigint                   not null,
    settlement_household_id bigint not null,
    member_id                               bigint                   default 0                                       not null,
    date                                    date                     default CURRENT_DATE                            not null,
    security_code                           varchar(24)              default ''::character varying                   not null,
    closing_price                           numeric(18, 4)           default 0                                       not null,
    total_shares                            bigint                   default 0                                       not null,
    cost                                    numeric(18, 4)           default 0                                       not null,
    average_unit_price_per_share            numeric(18, 4)           default 0                                       not null,
    market_value                            numeric(18, 4)           default 0                                       not null,
    ratio                                   numeric(18, 4)           default 0                                       not null,
    transfer_tax                            numeric(18, 4)           default 0                                       not null,
    profit_and_loss                         numeric(18, 4)           default 0                                       not null,
    profit_and_loss_percentage              numeric(18, 4)           default 0                                       not null,
    created_time                            timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time                            timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    previous_day_market_value               numeric(18, 4)           default 0                                       not null,
    previous_day_profit_and_loss            numeric(18, 4)           default 0                                       not null,
    previous_day_profit_and_loss_percentage numeric(18, 4)           default 0                                       not null,
    primary key (date, security_code, household_id, member_id)
);

comment on column public.daily_money_history_detail.household_id is '所屬家庭；所有寫入必須明確指定';
comment on column public.daily_money_history_detail.member_id is '資料屬於那一個會員；0 代表該家庭的合計';
comment on column public.daily_money_history_detail.date is '資料屬於那一天';
comment on column public.daily_money_history_detail.closing_price is '當日收盤價格';
comment on column public.daily_money_history_detail.total_shares is '總股數';
comment on column public.daily_money_history_detail.cost is '成本';
comment on column public.daily_money_history_detail.average_unit_price_per_share is '每股平均單價';
comment on column public.daily_money_history_detail.market_value is '市值';
comment on column public.daily_money_history_detail.ratio is '比重';
comment on column public.daily_money_history_detail.transfer_tax is '交易稅';
comment on column public.daily_money_history_detail.profit_and_loss is '參考損益';
comment on column public.daily_money_history_detail.profit_and_loss_percentage is '參考損益百分比';
comment on column public.daily_money_history_detail.previous_day_market_value is '前一次收盤時的市值';
comment on column public.daily_money_history_detail.previous_day_profit_and_loss is '與前一次收盤市值比較損益';
comment on column public.daily_money_history_detail.previous_day_profit_and_loss_percentage is '與前一次收盤市值比較損益百分比';

create index if not exists "daily_money_history_detail-security_code-idx"
    on public.daily_money_history_detail (security_code);


comment on column public.daily_money_history_detail.settlement_household_id is '結算時家庭；轉換家庭不改此欄位，用於刪除時重算歷史合計';
