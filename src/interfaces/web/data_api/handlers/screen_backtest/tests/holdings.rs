//! 條件選股回測的每期持股的選擇（緩衝、大盤濾網、續抱條件、產業上限）測試。

use super::*;

/// 不緩衝時每期取前 limit 檔；沒有報酬的股票占名額但不列入平均；沒有候選為空手。
#[test]
fn select_holdings_without_buffer_takes_top_limit() {
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(10.0)), ("B", None), ("C", Some(2.0))]),
            candidates(3, &[]),
            candidates(4, &[("D", None)]),
        ],
        2,
        2,
        None,
    );
    assert_eq!(rows[0].symbols, ["A", "B"]);
    assert_eq!((rows[0].holdings, rows[0].avg_return), (2, Some(10.0)));
    assert_eq!(
        (rows[0].benchmark_return, rows[0].etf_return),
        (Some(1.0), Some(0.5))
    );
    assert_eq!((rows[1].holdings, rows[1].avg_return), (0, None));
    assert!(rows[1].symbols.is_empty());
    assert_eq!((rows[2].holdings, rows[2].avg_return), (1, None));
}

/// 緩衝：上一期持股只要仍在前 keep_rank 名就續抱，空出的名額依排名補滿；輸出依排名排序。
#[test]
fn select_holdings_keeps_previous_within_buffer() {
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(1.0)), ("B", Some(2.0)), ("C", Some(3.0))]),
            // A 掉到第 4 名仍在緩衝內續抱；B 跌出名單賣出，補第 1 名 D。
            candidates(
                3,
                &[
                    ("D", Some(4.0)),
                    ("C", Some(5.0)),
                    ("E", Some(6.0)),
                    ("A", Some(7.0)),
                ],
            ),
            // A 掉到第 5 名超出緩衝：換成排名最高、尚未持有的 E。
            candidates(
                4,
                &[
                    ("C", Some(1.0)),
                    ("D", Some(1.0)),
                    ("E", Some(1.0)),
                    ("F", Some(1.0)),
                    ("A", Some(1.0)),
                ],
            ),
        ],
        2,
        4,
        None,
    );
    assert_eq!(rows[0].symbols, ["A", "B"]);
    assert_eq!(rows[1].symbols, ["D", "A"]);
    assert_eq!(rows[1].avg_return, Some(5.5));
    assert_eq!(rows[2].symbols, ["C", "D"]);
    // 續抱的股票比名額多時只留排名最前的 limit 檔。
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", None), ("B", None), ("C", None)]),
            candidates(3, &[("X", None), ("C", None), ("B", None), ("A", None)]),
        ],
        3,
        4,
        None,
    );
    assert_eq!(rows[1].symbols, ["C", "B", "A"]);
}

/// 大盤濾網沒過的那期空手並賣光；下一期重新依排名買進（不算續抱）。
#[test]
fn select_holdings_goes_to_cash_when_market_filtered() {
    let mut filtered = candidates(3, &[("A", Some(5.0)), ("B", Some(1.0))]);
    filtered.market_ok = false;
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(1.0)), ("B", Some(2.0))]),
            filtered,
            candidates(4, &[("C", Some(3.0)), ("A", Some(4.0))]),
        ],
        1,
        2,
        None,
    );
    assert_eq!(
        (rows[0].symbols.clone(), rows[0].market_filtered),
        (vec!["A".to_owned()], false)
    );
    assert!(rows[1].market_filtered && rows[1].symbols.is_empty());
    assert_eq!((rows[1].holdings, rows[1].avg_return), (0, None));
    // 上一期空手，A 雖在緩衝內也不是續抱，改買第 1 名 C。
    assert_eq!(rows[2].symbols, ["C"]);
    let summary = summarize(&rows, &free(&rows), 20);
    assert_eq!(summary.market_filtered_periods, 1);
}

/// 續抱條件：上一期持股不再符合選股條件、但相對強度仍達門檻就續抱，報酬取續抱名單的報酬；
/// 跌破門檻才換股。續抱股排在候選股之後，名額不足時先留候選股。
#[test]
fn select_holdings_keeps_stocks_meeting_hold_rule() {
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", Some(1.0)), ("B", Some(2.0))]),
            // A、B 都不再是候選（例如連買中斷）；A 的 RS 仍達門檻續抱，B 沒有，補第 1 名 C。
            with_hold(
                candidates(3, &[("C", Some(3.0)), ("D", Some(4.0))]),
                &[("C", Some(3.0)), ("A", Some(5.0))],
            ),
            // A 跌破門檻：換成 D。
            with_hold(
                candidates(4, &[("C", Some(1.0)), ("D", Some(2.0))]),
                &[("C", Some(1.0))],
            ),
        ],
        2,
        2,
        None,
    );
    assert_eq!(rows[1].symbols, ["C", "A"]);
    assert_eq!(rows[1].avg_return, Some(4.0));
    assert_eq!(rows[2].symbols, ["C", "D"]);

    // 續抱的股票多於名額時先留候選股。
    let rows = select_holdings(
        vec![
            candidates(2, &[("A", None), ("B", None)]),
            with_hold(
                candidates(3, &[("B", None), ("X", None)]),
                &[("A", None), ("B", None)],
            ),
        ],
        1,
        1,
        None,
    );
    assert_eq!(rows[0].symbols, ["A"]);
    // A 只符合續抱條件、B 是候選第 1 名但不是上一期持股：續抱 A。
    assert_eq!(rows[1].symbols, ["A"]);
}

/// 產業上限：同產業滿 N 檔後排名較後的跳過、由其他產業補上；續抱的股票也計入上限；
/// 沒有產業資料視為同一組（編號 0）。
#[test]
fn select_holdings_caps_stocks_per_industry() {
    // A、B、C 同產業 1，D 產業 2：上限 2 時取 A、B 後跳過 C，補 D。
    let rows = select_holdings(
        vec![with_industries(
            candidates(
                2,
                &[
                    ("A", Some(1.0)),
                    ("B", Some(2.0)),
                    ("C", Some(3.0)),
                    ("D", Some(4.0)),
                ],
            ),
            &[1, 1, 1, 2],
            &[],
        )],
        3,
        3,
        Some(2),
    );
    assert_eq!(rows[0].symbols, ["A", "B", "D"]);
    assert_eq!(rows[0].avg_return, Some(7.0 / 3.0));
    // 不設上限時就是前 3 名。
    let rows = select_holdings(
        vec![with_industries(
            candidates(2, &[("A", None), ("B", None), ("C", None), ("D", None)]),
            &[1, 1, 1, 2],
            &[],
        )],
        3,
        3,
        None,
    );
    assert_eq!(rows[0].symbols, ["A", "B", "C"]);

    // 續抱：上一期 A（產業 1）只符合續抱條件也計入上限，這期產業 1 只能再放 0 檔（上限 1），
    // 候選 B（產業 1）跳過，補 C（產業 3）。
    let rows = select_holdings(
        vec![
            with_industries(candidates(2, &[("A", None)]), &[1], &[]),
            with_hold(
                with_industries(candidates(3, &[("B", None), ("C", None)]), &[1, 3], &[1]),
                &[("A", None)],
            ),
        ],
        2,
        2,
        Some(1),
    );
    assert_eq!(rows[1].symbols, ["C", "A"]);

    // 沒有產業資料（舊資料或測試）視為同一組：上限 1 只放 1 檔。
    let rows = select_holdings(
        vec![candidates(2, &[("A", None), ("B", None)])],
        2,
        2,
        Some(1),
    );
    assert_eq!(rows[0].symbols, ["A"]);
}
