--DROP TABLE IF EXISTS daily_money_history;
create table public.daily_money_history
(
    date         date                     default CURRENT_DATE                            not null
        primary key,
    sum          numeric(18, 4)           default 0                                       not null,
    eddie        numeric(18, 4)           default 0                                       not null,
    unice        numeric(18, 4)           default 0                                       not null,
    created_time timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

comment on column public.daily_money_history.date is '資料屬於那一天';

create unique index "idx-daily_money_history-date"
    on public.daily_money_history (date desc);


