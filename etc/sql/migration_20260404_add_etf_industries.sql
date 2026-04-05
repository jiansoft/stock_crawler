-- Use internal stable ids for ETF-related industries instead of source-native codes.
insert into stock_industry (stock_industry_id, name)
values (9001, 'ETF')
    on conflict (stock_industry_id) do update set name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (9002, 'ETN')
    on conflict (stock_industry_id) do update set name = excluded.name;

insert into stock_industry (stock_industry_id, name)
values (9003, '受益證券')
    on conflict (stock_industry_id) do update set name = excluded.name;

-- Migrate existing stock records from old source-native ids to internal stable ids.
update stocks
set stock_industry_id = 9001
where stock_industry_id in (40, 140);

update stocks
set stock_industry_id = 9002
where stock_industry_id in (41, 141);

update stocks
set stock_industry_id = 9003
where stock_industry_id in (42, 142);

-- Remove old source-native ids after data migration.
delete from stock_industry
where stock_industry_id in (40, 41, 42, 140, 141, 142);
