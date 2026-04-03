create table if not exists public.daily_money_history_member
(
    date         date                     default CURRENT_DATE                            not null,
    member_id    bigint                   default 0                                       not null,
    market_value numeric(18, 4)           default 0                                       not null,
    created_time timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    primary key (date, member_id)
);

comment on column public.daily_money_history_member.date is '資料屬於那一天';
comment on column public.daily_money_history_member.member_id is '會員編號；0 代表全體總和';
comment on column public.daily_money_history_member.market_value is '該會員於當日收盤的市值總額';
comment on column public.daily_money_history_member.created_time is '資料建立時間';
comment on column public.daily_money_history_member.updated_time is '資料最後更新時間';

create index if not exists "daily_money_history_member-member_id-date-idx"
    on public.daily_money_history_member (member_id, date desc);
