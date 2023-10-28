create table public.company_word
(
    word_id      bigserial
        primary key,
    word         varchar(64)              default ''::character varying                   not null,
    created_time timestamp with time zone default ('now'::text)::timestamp with time zone not null,
    updated_time timestamp with time zone default ('now'::text)::timestamp with time zone not null
);

create unique index "company_word-word-idx"
    on public.company_word (word) include (word_id);