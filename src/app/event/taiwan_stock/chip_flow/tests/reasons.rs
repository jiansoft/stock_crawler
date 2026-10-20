//! 連續買賣超天數與通知理由（法人大額買賣、連續天數、融資增減）的判斷。

use super::*;

/// 同向累加、反向重新起算、沒有買賣超歸零；同一天重跑維持原值。
#[test]
fn next_streak_counts_consecutive_days() {
    let first = next_streak(None, day(5), Some(flow(1_000, -500, 0)));
    assert_eq!((first.foreign, first.trust), (1, -1));

    let second = next_streak(Some(first), day(6), Some(flow(2_000, -100, 0)));
    assert_eq!((second.foreign, second.trust), (2, -2));
    assert_eq!(
        next_streak(Some(second), day(6), Some(flow(-1, 1, 0))),
        second
    );

    let third = next_streak(Some(second), day(7), Some(flow(-3_000, 0, 0)));
    assert_eq!((third.foreign, third.trust), (-1, 0));
    let fourth = next_streak(Some(third), day(8), None);
    assert_eq!((fourth.foreign, fourth.trust), (0, 0));
}

/// 外資、投信的大量條件要同時達到比例與張數；沒有成交量時不成立。
#[test]
fn reasons_flag_heavy_institutional_flows() {
    let streak = Streak::default();
    // 2330 2026-10-05：外資 9,771 張佔成交 25,987 張的 37.6%；投信 2.3% 未達
    let tsmc = report(
        Some(flow(9_770_520, 589_108, 491_655)),
        None,
        Some(25_987_000),
        streak,
    );
    assert_eq!(reasons(&tsmc), vec!["外資大量"]);

    let small = report(
        Some(flow(900_000, 150_000, 0)),
        None,
        Some(2_000_000),
        streak,
    );
    assert!(
        reasons(&small).is_empty(),
        "外資比例夠但張數未達、投信比例未達"
    );

    let trust = report(Some(flow(0, 300_000, 0)), None, Some(1_500_000), streak);
    assert_eq!(reasons(&trust), vec!["投信大量"]);

    let no_volume = report(Some(flow(9_770_520, 0, 0)), None, None, streak);
    assert!(reasons(&no_volume).is_empty());
}

/// 連續天數達門檻就列入，不論方向。
#[test]
fn reasons_flag_streaks() {
    let streak = Streak {
        date: day(6),
        foreign: -5,
        trust: 3,
    };
    assert_eq!(
        reasons(&report(None, None, None, streak)),
        vec!["外資連續", "投信連續"]
    );
}

/// 融資、融券餘額增減要同時達到比例與張數門檻；前日為 0 時只看張數。
#[test]
fn reasons_flag_margin_changes() {
    let margin = |margin_previous, margin_today, short_previous, short_today| MarginBalance {
        margin_previous,
        margin_today,
        short_previous,
        short_today,
    };
    let streak = Streak::default();
    assert_eq!(
        reasons(&report(
            None,
            Some(margin(5_000, 5_600, 300, 520)),
            None,
            streak
        )),
        vec!["融資", "融券"]
    );
    assert!(
        reasons(&report(
            None,
            Some(margin(30_939, 30_135, 18, 46)),
            None,
            streak
        ))
        .is_empty(),
        "融資 −2.6% 未達比例、融券 +28 張未達張數"
    );
    assert!(changed(0, 500, MARGIN_MIN_PERCENT, MARGIN_MIN_LOTS));
    assert!(!changed(0, 499, MARGIN_MIN_PERCENT, MARGIN_MIN_LOTS));
}
