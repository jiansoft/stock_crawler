-- 將舊版扁平表 daily_money_history 的歷史資料搬到垂直表 daily_money_history_member。
--
-- 注意：
-- 1. 此 SQL 只能完整還原舊表中已存在的三個 bucket：
--    - member_id = 0 -> sum
--    - member_id = 1 -> eddie
--    - member_id = 2 -> unice
-- 2. 舊表的 unice 是歷史上「非 Eddie」的聚合欄位；
--    若某些日期實際存在 2 號以外的會員，舊表本身已無法再拆回各自 member_id。

insert into daily_money_history_member (
    date,
    member_id,
    market_value
)
select
    migrated.date,
    migrated.member_id,
    migrated.market_value
from (
    select date, 0::bigint as member_id, sum as market_value
    from daily_money_history

    union all

    select date, 1::bigint as member_id, eddie as market_value
    from daily_money_history

    union all

    select date, 2::bigint as member_id, unice as market_value
    from daily_money_history
) as migrated
on conflict (date, member_id) do update set
    market_value = excluded.market_value,
    updated_time = now();
