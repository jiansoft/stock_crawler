create table if not exists public.sell_history
(
    serial                    bigserial
        primary key,
    member_id                 bigint                   default 0                                       not null,
    date                      date                     default CURRENT_DATE                            not null,
    security_code             varchar(24)              default ''::character varying                   not null,
    purchase_amount_per_share numeric(18, 4)           default 0                                       not null,
    cost                      numeric(18, 4)           default 0                                       not null,
    sell_amount_per_share     numeric(18, 4)           default 0                                       not null,
    number_of_shares_traded   numeric(18, 4)           default 0                                       not null,
    amount                    numeric(18, 4)           default 0                                       not null,
    income                    numeric(18, 4)           default 0                                       not null,
    create_time               timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    update_time               timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    is_reverted               boolean                  default false                                   not null,
    reverted_time             timestamp with time zone
);

comment on table public.sell_history is '賣出紀錄（由持股管理介面寫入；stock_rust 只讀，用於持股績效的賣出金流）';
comment on column public.sell_history.member_id is '會員編號 Member.Id';
comment on column public.sell_history.date is '資料屬於那一天';
comment on column public.sell_history.security_code is '股票代碼';
comment on column public.sell_history.purchase_amount_per_share is '每股買入平均單價';
comment on column public.sell_history.cost is '原始成本 = 交易股數 * 每股買入平均單價';
comment on column public.sell_history.sell_amount_per_share is '每股賣出單價';
comment on column public.sell_history.number_of_shares_traded is '交易股數';
comment on column public.sell_history.amount is '交易金額';
comment on column public.sell_history.income is '損益 = 交易金額 - 原始成本';
comment on column public.sell_history.is_reverted is '是否已撤銷（撤銷的賣出不計入金流）';

create index if not exists sell_history_is_reverted_idx
    on public.sell_history (is_reverted, date desc);
