use jiff::civil::{Weekday, date};
use jiff::{SignedDuration, ToSpan};

use super::*;

fn taipei_at(seconds: i64, nanos: i32) -> TaipeiDateTime {
    TaipeiDateTime::from_timestamp(Timestamp::new(seconds, nanos).expect("時間點合法"))
}

#[test]
fn test_formats_match_chrono_for_every_fraction_width() {
    // 期望值取自 chrono `DateTime<FixedOffset>`（+08:00）的實際輸出，移除 chrono 前逐一比對過。
    // 2026-11-22T05:05:09Z = 臺灣 13:05:09；涵蓋沒有小數、3、6、9 位與跨日的情況。
    let cases: [(i64, i32, &str, &str); 6] = [
        (1_795_323_909, 0, "2026-11-22", "13:05:09"),
        (1_795_323_909, 120_000_000, "2026-11-22", "13:05:09.120"),
        (1_795_323_909, 123_456_000, "2026-11-22", "13:05:09.123456"),
        (
            1_795_323_909,
            123_456_789,
            "2026-11-22",
            "13:05:09.123456789",
        ),
        (1_795_323_909, 5, "2026-11-22", "13:05:09.000000005"),
        (1_795_366_799, 999_999_000, "2026-11-23", "00:59:59.999999"),
    ];
    for (seconds, nanos, day, clock) in cases {
        let ours = taipei_at(seconds, nanos);
        assert_eq!(
            ours.to_string(),
            format!("{day} {clock} +08:00"),
            "Display 沿用 chrono 格式"
        );
        assert_eq!(
            format!("{ours:?}"),
            format!("{day}T{clock}+08:00"),
            "Debug 沿用 chrono 格式"
        );
        assert_eq!(
            serde_json::to_string(&ours).expect("序列化"),
            format!("\"{day}T{clock}+08:00\""),
            "JSON 沿用 chrono 格式"
        );
        assert_eq!(ours.to_rfc3339(), format!("{day}T{clock}+08:00"));
    }
}

#[test]
fn test_fields_are_read_in_taipei_time() {
    // 2026-11-22T17:30:00Z 在臺灣已是 23 日 01:30
    let moment: TaipeiDateTime = "2026-11-22T17:30:00Z".parse().expect("解析");
    assert_eq!(moment.date(), date(2026, 11, 23));
    assert_eq!(moment.datetime().hour(), 1);
    assert_eq!(moment.time().minute(), 30);
    assert_eq!(moment.date().weekday(), Weekday::Monday);
    assert_eq!(
        moment.strftime("%Y-%m-%d %H:%M:%S %:z").to_string(),
        "2026-11-23 01:30:00 +08:00"
    );
    assert_eq!(
        TaipeiDateTime::from_datetime(date(2026, 11, 23).at(1, 30, 0, 0)).expect("合法"),
        moment,
        "本地日期時間以 +8 解讀"
    );
}

#[test]
fn test_serde_round_trip_and_accepts_other_offsets() {
    let moment = taipei_at(1_795_323_909, 123_456_000);
    let json = serde_json::to_string(&moment).expect("序列化");
    assert_eq!(json, "\"2026-11-22T13:05:09.123456+08:00\"");
    assert_eq!(
        serde_json::from_str::<TaipeiDateTime>(&json).expect("還原"),
        moment
    );
    let from_utc: TaipeiDateTime =
        serde_json::from_str("\"2026-11-22T05:05:09.123456Z\"").expect("UTC 字串");
    assert_eq!(from_utc, moment, "同一時間點不論偏移都相等");
    assert!(
        serde_json::from_str::<TaipeiDateTime>("\"2026-11-22\"").is_err(),
        "沒有時刻不接受"
    );
}

#[test]
fn test_arithmetic_and_ordering() {
    let start = taipei_at(1_795_323_909, 0);
    let later = start + SignedDuration::from_mins(90);
    assert!(later > start);
    assert_eq!(later - start, SignedDuration::from_mins(90));
    assert_eq!(later - SignedDuration::from_mins(90), start);
    let mut moving = start;
    moving += SignedDuration::from_secs(1);
    moving -= SignedDuration::from_secs(1);
    assert_eq!(moving, start);
    assert_eq!(
        start
            .checked_add(SignedDuration::from_hours(11))
            .expect("合法")
            .date(),
        date(2026, 11, 23)
    );
    // 1 月 31 日加一個月：依日曆停在 2 月底
    let end_of_january =
        TaipeiDateTime::from_datetime(date(2026, 1, 31).at(9, 0, 0, 0)).expect("合法");
    assert_eq!(
        end_of_january
            .checked_add_span(1.month())
            .expect("合法")
            .date(),
        date(2026, 2, 28)
    );
    assert_eq!(Timestamp::from(start), start.timestamp());
    assert_eq!(TaipeiDateTime::from(start.timestamp()), start);
    assert_eq!(
        TaipeiDateTime::default().timestamp(),
        Timestamp::UNIX_EPOCH,
        "預設值與 chrono 相同為 1970 紀元"
    );
}

#[test]
fn test_ymd_matches_chrono_from_ymd_opt() {
    // 期望值與 chrono `NaiveDate::from_ymd_opt` 相同：不存在的日期與超出範圍的月日回傳 None
    let cases: [(i32, u32, u32, Option<&str>); 6] = [
        (2026, 11, 22, Some("2026-11-22")),
        (2024, 2, 29, Some("2024-02-29")),
        (2026, 2, 29, None),
        (2026, 13, 1, None),
        (2026, 0, 1, None),
        (2026, 4, 31, None),
    ];
    for (year, month, day, expected) in cases {
        let ours = ymd(year, month, day).map(|value| value.to_string());
        assert_eq!(
            ours.as_deref(),
            expected,
            "{year}-{month}-{day} 的結果要與 chrono 相同"
        );
    }
    // 已知差異：jiff 的年份只到 9999（chrono 到 262143），超出時回傳 None
    assert_eq!(ymd(10_000, 1, 1), None);
    assert_eq!(ymd(9_999, 12, 31), Some(date(9999, 12, 31)));
}

#[test]
fn test_today_and_now_are_taipei_time() {
    let now = TaipeiDateTime::now();
    let system_now = Timestamp::now();
    assert!((now.timestamp().as_second() - system_now.as_second()).abs() <= 2);
    // 同一時間點加 8 小時後的 UTC 日期時間，就是臺灣的日期時間
    let shifted_utc =
        tz::TimeZone::UTC.to_datetime(now.timestamp() + SignedDuration::from_hours(8));
    assert_eq!(now.datetime(), shifted_utc, "臺灣時間固定為 UTC+8");
    assert_eq!(now.date(), shifted_utc.date());
    assert!(today() >= now.date());
    assert_eq!(taipei().to_offset(now.timestamp()), tz::offset(8));
}
