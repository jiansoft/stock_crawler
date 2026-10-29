//! HiStock 報價列一致性檢查與整批篩選的測試。

use super::*;

/// 欄位彼此吻合的列不會被判為矛盾；尚未成交的列不檢查。
#[test]
fn row_inconsistency_accepts_consistent_and_untraded_rows() {
    assert_eq!(row_inconsistency(&consistent_snapshot("1301"), None), None);

    let mut untraded = consistent_snapshot("1301");
    untraded.price = Decimal::ZERO;
    assert_eq!(row_inconsistency(&untraded, None), None);

    // 平盤列：漲跌「--」解析為 0，成交價等於昨收。
    let mut flat = consistent_snapshot("6584");
    flat.price = dec!(75.4);
    flat.change = Decimal::ZERO;
    flat.low = dec!(75);
    assert_eq!(row_inconsistency(&flat, None), None);

    // 沒有最高最低與昨收（例如剛開盤欄位還是「--」）時無從比對，視為吻合。
    let mut sparse = RealtimeSnapshot::new("2486".to_string(), dec!(236));
    sparse.change = dec!(-4);
    assert_eq!(row_inconsistency(&sparse, None), None);
}

/// 10-08 開盤實例：1301 成交欄變成 33，但昨收與最高最低仍是自己的。
#[test]
fn row_inconsistency_rejects_price_outside_high_low() {
    let mut snapshot = consistent_snapshot("1301");
    snapshot.price = dec!(33);
    assert_eq!(
        row_inconsistency(&snapshot, None),
        Some(Inconsistency::OutOfRange)
    );
}

/// 成交價在高低之間但與漲跌對不上（錯在漲跌停範圍內）也要抓出來。
#[test]
fn row_inconsistency_rejects_price_that_disagrees_with_change() {
    let mut snapshot = consistent_snapshot("1301");
    snapshot.price = dec!(77);
    assert_eq!(
        row_inconsistency(&snapshot, None),
        Some(Inconsistency::ChangeMismatch)
    );
}

/// 除權息當天漲跌以參考價為準：成交價減漲跌等於參考價也算吻合。
#[test]
fn row_inconsistency_accepts_change_against_reference_price() {
    let mut snapshot = consistent_snapshot("2886");
    snapshot.price = dec!(40);
    snapshot.change = dec!(0.5);
    snapshot.last_close = dec!(41);
    snapshot.high = dec!(40.5);
    snapshot.low = dec!(39.2);

    assert_eq!(
        row_inconsistency(&snapshot, None),
        Some(Inconsistency::ChangeMismatch)
    );
    assert_eq!(row_inconsistency(&snapshot, Some(dec!(39.5))), None);
}

/// 少數矛盾列只把該列價格歸零（交給快取保留舊值），其餘照常使用。
#[test]
fn screen_inconsistent_rows_zeroes_only_bad_rows() {
    let mut bad = consistent_snapshot("79971");
    bad.price = dec!(33);
    let mut result = fetch_result_with(vec![
        consistent_snapshot("79970"),
        bad,
        consistent_snapshot("79972"),
    ]);

    let removed = screen_inconsistent_rows(&mut result, |_| RowBaseline::default()).unwrap();

    assert_eq!(removed, 1);
    assert_eq!(result.snapshots.len(), 3);
    assert_eq!(result.snapshots["79971"].price, Decimal::ZERO);
    assert_eq!(result.snapshots["79970"].price, dec!(78.5));
    assert_eq!(result.snapshots["79972"].price, dec!(78.5));
}

/// 矛盾列超過 max(20, 2%) 代表整頁錯亂，整批捨棄。
#[test]
fn screen_inconsistent_rows_rejects_dirty_batch() {
    let snapshots = (0..30)
        .map(|i| {
            let mut snapshot = consistent_snapshot(&format!("7990{i:02}"));
            if i < 21 {
                snapshot.price = dec!(33);
            }
            snapshot
        })
        .collect();
    let mut result = fetch_result_with(snapshots);

    let error = screen_inconsistent_rows(&mut result, |_| RowBaseline::default()).unwrap_err();

    let dirty = error.downcast_ref::<DirtyBatch>().expect("DirtyBatch 錯誤");
    assert_eq!(dirty.inconsistent, 21);
    assert_eq!(dirty.total, 30);
    assert!(error.to_string().contains("21/30"));
}

/// 剛好 20 列矛盾還在容許範圍內，只剔除那幾列。
#[test]
fn screen_inconsistent_rows_tolerates_up_to_limit() {
    let snapshots = (0..30)
        .map(|i| {
            let mut snapshot = consistent_snapshot(&format!("7990{i:02}"));
            if i < 20 {
                snapshot.price = dec!(33);
            }
            snapshot
        })
        .collect();
    let mut result = fetch_result_with(snapshots);

    assert_eq!(
        screen_inconsistent_rows(&mut result, |_| RowBaseline::default()).unwrap(),
        20
    );
}

/// 真實頁面形狀的 fixture 不應有任何列被判為矛盾。
#[test]
fn screen_inconsistent_rows_keeps_fixture_page_intact() {
    const FIXTURE: &str = include_str!("../../testdata/rank_page.html");
    let mut result = parse_rank_html(FIXTURE).unwrap();

    assert_eq!(
        screen_inconsistent_rows(&mut result, |_| RowBaseline::default()).unwrap(),
        0
    );
    assert!(result.snapshots.values().all(|s| s.price > Decimal::ZERO));
}

/// 同一檔當天第二次前後矛盾只記 debug，但仍照樣剔除該列。
#[test]
fn screen_inconsistent_rows_zeroes_repeated_bad_row() {
    for _ in 0..2 {
        let mut bad = consistent_snapshot("79975");
        bad.price = dec!(33);
        let mut result = fetch_result_with(vec![bad, consistent_snapshot("79976")]);

        let removed = screen_inconsistent_rows(&mut result, |_| RowBaseline::default()).unwrap();

        assert_eq!(removed, 1);
        assert_eq!(result.snapshots["79975"].price, Decimal::ZERO);
        assert_eq!(result.snapshots["79976"].price, dec!(78.5));
    }
}

/// 排行頁 fixture 經解析與篩除後全部保留。
#[test]
fn screen_rank_page_keeps_fixture_rows() {
    const FIXTURE: &str = include_str!("../../testdata/rank_page.html");

    let result = screen_rank_page(FIXTURE).unwrap();

    assert!(!result.snapshots.is_empty());
    assert!(result.snapshots.values().all(|s| s.price > Decimal::ZERO));
}

/// 以快取昨收建立比對基準。
fn baseline(last_close: Decimal) -> RowBaseline {
    RowBaseline {
        last_close: Some(last_close),
        reference_price: None,
    }
}

/// 整列換成別檔（10-28 09:21 實例：4979 列上的昨收約 89，快取昨收 467.5）：欄位彼此吻合，
/// 只有和快取昨收比對才抓得到；差距在 25% 以內（例如快取昨收舊了兩天）不算。
#[test]
fn misaligned_row_detects_rows_from_another_stock() {
    let row = consistent_snapshot("4979");
    assert_eq!(row_inconsistency(&row, None), None);
    assert!(misaligned_row(&row, baseline(dec!(467.5))));
    assert!(misaligned_row(&row, baseline(dec!(45))));

    // 昨收一致或差距在 25% 以內：不是別檔。
    assert!(!misaligned_row(&row, baseline(dec!(75.4))));
    assert!(!misaligned_row(&row, baseline(dec!(62))));
    assert!(!misaligned_row(&row, baseline(dec!(93))));

    // 除權息當天列上的昨收是參考價，等於參考價就不算。
    let ex_rights = RowBaseline {
        last_close: Some(dec!(120)),
        reference_price: Some(dec!(75.4)),
    };
    assert!(!misaligned_row(&row, ex_rights));

    // 沒有快取昨收（新股、興櫃）、列上沒有昨收或尚未成交時不檢查。
    assert!(!misaligned_row(&row, RowBaseline::default()));
    let mut no_close = consistent_snapshot("4979");
    no_close.last_close = Decimal::ZERO;
    assert!(!misaligned_row(&no_close, baseline(dec!(467.5))));
    let mut untraded = consistent_snapshot("4979");
    untraded.price = Decimal::ZERO;
    assert!(!misaligned_row(&untraded, baseline(dec!(467.5))));
}

/// 一兩列別檔只剔除那幾列；3 列以上代表整頁錯位，整批捨棄（即使遠低於 max(20, 2%)）。
#[test]
fn screen_inconsistent_rows_rejects_misaligned_batch() {
    let snapshots = || {
        (0..30)
            .map(|i| consistent_snapshot(&format!("7990{i:02}")))
            .collect()
    };
    let shifted = |count: usize| {
        move |symbol: &str| {
            let index: usize = symbol[4..].parse().unwrap();
            baseline(if index < count { dec!(300) } else { dec!(75.4) })
        }
    };

    let mut result = fetch_result_with(snapshots());
    assert_eq!(
        screen_inconsistent_rows(&mut result, shifted(2)).unwrap(),
        2
    );
    assert_eq!(result.snapshots["799000"].price, Decimal::ZERO);
    assert_eq!(result.snapshots["799002"].price, dec!(78.5));

    let mut result = fetch_result_with(snapshots());
    let error = screen_inconsistent_rows(&mut result, shifted(3)).unwrap_err();
    let dirty = error.downcast_ref::<DirtyBatch>().expect("DirtyBatch 錯誤");
    assert_eq!(
        (dirty.inconsistent, dirty.misaligned, dirty.total),
        (3, 3, 30)
    );
    assert_eq!(dirty.samples.len(), 3);
    let message = error.to_string();
    assert!(
        message.contains(
            "3/30 列前後矛盾（高低 0、漲跌 0、別檔 3），整批捨棄；例：799000 昨收與快取昨收差距過大"
        ),
        "{message}"
    );
}
