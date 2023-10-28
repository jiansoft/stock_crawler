create table public.company_index
(
    word_id       bigint                   default 0                                       not null,
    security_code varchar(24)              default ''::character varying                   not null,
    created_time  timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time  timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    primary key (word_id, security_code)
);
