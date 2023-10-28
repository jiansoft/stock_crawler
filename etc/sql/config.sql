create table public.config
(
    key varchar(64)  default ''::character varying not null
        primary key,
    val varchar(256) default ''::character varying
);