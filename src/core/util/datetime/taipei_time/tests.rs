use chrono::{FixedOffset, TimeZone as _};
use jiff::civil::{Weekday, date};
use jiff::{SignedDuration, ToSpan};

use super::*;

/// 以 chrono 在 UTC+8 下的 `DateTime` 作為對照（chrono 移除前用來確認輸出逐字相同）。
fn chrono_reference(seconds: i64, nanos: u32) -> chrono::DateTime<FixedOffset> {
    FixedOffset::east_opt(8 * 3600)
        .expect("固定偏移 +8 合法")
        .timestamp_opt(seconds, nanos)
        .single()
        .expect("時間點合法")
}

fn taipei_at(seconds: i64, nanos: i32) -> TaipeiDateTime {
    TaipeiDateTime::from_timestamp(Timestamp::new(seconds, nanos).expect("時間點合法"))
}

#[test]
fn test_formats_match_chrono_for_every_fraction_width() {
    // 2026-11-22T05:05:09Z = 臺灣 13:05:09；涵蓋沒有小數、3、6、9 位與跨日的情況
    let cases: [(i64, u32); 6] = [
        (1_795_323_909, 0),
        (1_795_323_909, 120_000_000),
        (1_795_323_909, 123_456_000),
        (1_795_323_909, 123_456_789),
        (1_795_323_909, 5),
        (1_795_366_799, 999_999_000),
    ];
    for (seconds, nanos) in cases {
        let ours = taipei_at(seconds, i32::try_from(nanos).expect("奈秒在範圍內"));
        let chrono = chrono_reference(seconds, nanos);
        assert_eq!(
            ours.to_string(),
            chrono.to_string(),
            "Display 要與 chrono 相同"
        );
        assert_eq!(
            format!("{ours:?}"),
            format!("{chrono:?}"),
            "Debug 要與 chrono 相同"
        );
        assert_eq!(
            serde_json::to_string(&ours).expect("序列化"),
            serde_json::to_string(&chrono).expect("序列化"),
            "JSON 要與 chrono 相同"
        );
        assert_eq!(ours.to_string(), format!("{ours}"));
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
    let cases: [(i32, u32, u32); 6] = [
        (2026, 11, 22),
        (2024, 2, 29),
        (2026, 2, 29),
        (2026, 13, 1),
        (2026, 0, 1),
        (2026, 4, 31),
    ];
    for (year, month, day) in cases {
        let ours = ymd(year, month, day).map(|value| value.to_string());
        let chrono =
            chrono::NaiveDate::from_ymd_opt(year, month, day).map(|value| value.to_string());
        assert_eq!(ours, chrono, "{year}-{month}-{day} 的結果要與 chrono 相同");
    }
    // 已知差異：jiff 的年份只到 9999（chrono 到 262143），超出時回傳 None
    assert_eq!(ymd(10_000, 1, 1), None);
    assert!(chrono::NaiveDate::from_ymd_opt(10_000, 1, 1).is_some());
}

#[test]
fn test_today_and_now_are_taipei_time() {
    let now = TaipeiDateTime::now();
    let expected = chrono::Utc::now().with_timezone(&FixedOffset::east_opt(8 * 3600).expect("+8"));
    assert!((now.timestamp().as_second() - expected.timestamp()).abs() <= 2);
    assert_eq!(now.date().to_string(), expected.date_naive().to_string());
    assert!(today() >= now.date());
    assert_eq!(taipei().to_offset(now.timestamp()), tz::offset(8));
}
