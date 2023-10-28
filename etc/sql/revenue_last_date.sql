create table public.revenue_last_date
(
    security_code varchar(64)              default ''::character varying                   not null
        primary key,
    serial        bigint                   default 0                                       not null,
    created_time  timestamp with time zone default ('now'::text)::timestamp with time zone not null
);