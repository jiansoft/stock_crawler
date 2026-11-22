use serde::{Deserialize, Serialize};

use super::*;

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Wrapper {
    #[serde(with = "super")]
    at: Timestamp,
}

#[test]
fn matches_chrono_utc_format() {
    let cases: [(i64, i32, &str); 5] = [
        (1_795_323_909, 0, "2026-11-22T05:05:09Z"),
        (1_795_323_909, 120_000_000, "2026-11-22T05:05:09.120Z"),
        (1_795_323_909, 123_456_000, "2026-11-22T05:05:09.123456Z"),
        (1_795_323_909, 123_456_789, "2026-11-22T05:05:09.123456789Z"),
        (1_795_323_909, 5, "2026-11-22T05:05:09.000000005Z"),
    ];
    for (seconds, nanos, expected) in cases {
        let at = Timestamp::new(seconds, nanos).expect("合法");
        let json = serde_json::to_string(&Wrapper { at }).expect("序列化");
        assert_eq!(
            json,
            format!("{{\"at\":\"{expected}\"}}"),
            "與 chrono 的 DateTime<Utc> 相同"
        );
        assert_eq!(serde_json::from_str::<Wrapper>(&json).expect("還原").at, at);
    }
    let from_taipei: Wrapper =
        serde_json::from_str("{\"at\":\"2026-11-22T13:05:09+08:00\"}").expect("其他偏移");
    assert_eq!(
        from_taipei.at,
        Timestamp::new(1_795_323_909, 0).expect("合法")
    );
}
