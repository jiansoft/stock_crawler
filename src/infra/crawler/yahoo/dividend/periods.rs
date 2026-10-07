//! 同一發放年度內的期別整理：重複期別的去重或改標，以及整年度配發併回最後一次除權息。

use std::collections::{HashMap, HashSet};

use super::YahooDividendDetail;

/// 把 Yahoo 另立一列的「整年度配發」併回同一所屬年度的最後一次除權息。
///
/// 整年度一次發放的股票股利，Yahoo 會拆成獨立一列（所屬期間只有年份、沒有 H/Q），
/// 但 Goodinfo 的除權息日程是掛在該所屬年度最後一次配發上——例如 2753 的 0.5 元
/// 就掛在 25H2 的 7.5 元現金旁邊。兩邊描述的是同一次配發，拆成兩列會讓年度合計
/// 把同一年的股利算成兩個事件，畫面上的全年合計也會對不起來。
///
/// 只有同一發放年度、同一所屬年度，且金額欄位不互相衝突（併入方該欄為零）時才合併；
/// 無法確定是同一次配發時保留原本的獨立列，交給 A 代碼處理，例如 2072 的全年列
/// 所屬年度與同年的 H1 不同，就不會被併走。
pub(super) fn merge_annual_event_into_last_period(details: &mut Vec<YahooDividendDetail>) {
    let annual_positions: Vec<usize> = details
        .iter()
        .enumerate()
        .filter(|(_, detail)| detail.quarter.is_empty())
        .map(|(index, _)| index)
        .collect();

    let mut merged_positions: Vec<usize> = Vec::new();
    for annual_index in annual_positions {
        let year_of_dividend = details[annual_index].year_of_dividend;
        // 期間代碼字典序即時間序（H2 > H1、Q4 > Q3），取最後一次配發作為併入目標。
        let Some(target_index) = details
            .iter()
            .enumerate()
            .filter(|(index, detail)| {
                *index != annual_index
                    && detail.year_of_dividend == year_of_dividend
                    && !detail.quarter.is_empty()
            })
            .max_by(|(_, left), (_, right)| left.quarter.cmp(&right.quarter))
            .map(|(index, _)| index)
        else {
            continue;
        };

        let annual = details[annual_index].clone();
        // 兩邊同一欄都有金額時無法判斷是同一次配發還是兩次，保留原狀比併錯安全。
        let conflicts = {
            let target = &details[target_index];
            (!annual.cash_dividend.is_zero() && !target.cash_dividend.is_zero())
                || (!annual.stock_dividend.is_zero() && !target.stock_dividend.is_zero())
        };
        if conflicts {
            continue;
        }

        let target = &mut details[target_index];
        if target.cash_dividend.is_zero() {
            target.cash_dividend = annual.cash_dividend;
        }
        if target.stock_dividend.is_zero() {
            target.stock_dividend = annual.stock_dividend;
        }
        merge_date(&mut target.ex_dividend_date1, &annual.ex_dividend_date1);
        merge_date(&mut target.ex_dividend_date2, &annual.ex_dividend_date2);
        merge_date(&mut target.payable_date1, &annual.payable_date1);
        merge_date(&mut target.payable_date2, &annual.payable_date2);

        merged_positions.push(annual_index);
    }

    // 由後往前刪，前面元素的索引才不會被移動。
    merged_positions.sort_unstable_by(|a, b| b.cmp(a));
    for index in merged_positions {
        details.remove(index);
    }
}

/// 處理同一發放年度內「所屬年度與期別都相同」的重複列。
///
/// 資料表的主鍵是 `(代號, 發放年度, 所屬年度, 期別)`，Yahoo 卻常給出重複的期別：
/// - 完全相同的兩列（00758B 2023Q4）：只留一列。
/// - ETF 的季別重複（00777B 每月配息、Yahoo 一季標三次 `2026Q2`）：依除權息日先後改成該季的月別，
///   Q2 的三次依序為 M04、M05、M06。
/// - ETF 的半年別重複（006208 一年兩次都標 `2025H1`）：依除權息日先後改成 H1、H2。
/// - ETF 的期別只有年份且重複（00930 雙月配每次都標 `2026`）：以除權息月份標成 M01～M12；
///   沒有除權息日的列略過，同月份有兩次時無法標記，保留日期最晚的一列。
/// - 個股的重複是同一次配息被列了兩次（延後除息前的舊日期仍掛著，如 1109 2022 年
///   07-14 與 08-04）：只留除權息日最晚的一列；抽查 10 檔有 9 檔與交易所除權息結果相符，
///   其餘由交易所資料核對流程修正。
pub(super) fn resolve_repeated_periods(details: &mut Vec<YahooDividendDetail>, is_etf: bool) {
    // 1. 完全相同的列
    let mut seen = HashSet::new();
    details.retain(|detail| {
        seen.insert((
            detail.year_of_dividend,
            detail.quarter.clone(),
            detail.cash_dividend,
            detail.stock_dividend,
            detail.ex_dividend_date1.clone(),
            detail.ex_dividend_date2.clone(),
        ))
    });

    // 2. 依 (所屬年度, 期別) 分組，只處理重複的組
    let mut groups: HashMap<(i32, String), Vec<usize>> = HashMap::new();
    for (index, detail) in details.iter().enumerate() {
        groups
            .entry((detail.year_of_dividend, detail.quarter.clone()))
            .or_default()
            .push(index);
    }

    let mut removed: Vec<usize> = Vec::new();
    for ((_, quarter), mut indexes) in groups {
        if indexes.len() < 2 {
            continue;
        }
        indexes.sort_by_key(|&index| latest_event_date(&details[index]));

        if !is_etf {
            // 個股：保留日期最晚的一列。
            removed.extend(indexes.iter().take(indexes.len() - 1).copied());
            continue;
        }

        let relabeled: Option<Vec<String>> = match quarter.as_str() {
            "Q1" | "Q2" | "Q3" | "Q4" if indexes.len() <= 3 => {
                let first_month = (u32::from(quarter.as_bytes()[1] - b'0') - 1) * 3 + 1;
                Some(
                    (0..indexes.len())
                        .map(|offset| format!("M{:02}", first_month + offset as u32))
                        .collect(),
                )
            }
            "H1" | "H2" if indexes.len() == 2 => Some(vec!["H1".to_string(), "H2".to_string()]),
            // 期別只有年份（00930 雙月配 Yahoo 每次都只寫「2026」）：沒有期間可用，改以除息月份標記。
            "" => {
                // 沒有除息日的列無從決定月份，先略過，等日期公布再收；留著它日後會變成重複列。
                let (dated, undated): (Vec<usize>, Vec<usize>) = indexes
                    .iter()
                    .partition(|&&index| !latest_event_date(&details[index]).is_empty());
                let months: Vec<String> = dated
                    .iter()
                    .map(|&index| format!("M{}", &latest_event_date(&details[index])[5..7]))
                    .collect();
                let distinct: HashSet<&String> = months.iter().collect();
                if dated.len() >= 2 && distinct.len() == months.len() {
                    removed.extend(undated);
                    indexes = dated;
                    Some(months)
                } else {
                    None
                }
            }
            _ => None,
        };
        match relabeled {
            Some(labels) => {
                for (index, label) in indexes.iter().zip(labels) {
                    details[*index].quarter = label;
                }
            }
            // 無法判斷怎麼拆時保留日期最晚的一列，總比讓後寫入的覆蓋先寫入的好。
            None => removed.extend(indexes.iter().take(indexes.len() - 1).copied()),
        }
    }

    removed.sort_unstable_by(|a, b| b.cmp(a));
    for index in removed {
        details.remove(index);
    }
}

/// 一筆配息最晚的實際除權息日；兩欄都沒有日期時回傳空字串（排在最前面）。
fn latest_event_date(detail: &YahooDividendDetail) -> String {
    [&detail.ex_dividend_date1, &detail.ex_dividend_date2]
        .into_iter()
        .filter(|date| is_actual_date(date))
        .max()
        .cloned()
        .unwrap_or_default()
}

/// 併入日期欄位：只有目標欄還沒有實際日期時才採用來源日期。
///
/// 「尚未公布」代表 Yahoo 已宣告但日期未定，仍屬於未知，可以被實際日期取代；
/// 反之目標欄已經是實際日期就不覆蓋，避免把確定的除權息日換成另一次配發的日期。
fn merge_date(target: &mut String, source: &str) {
    if source == "-" || source.is_empty() || source == *target {
        return;
    }
    if is_actual_date(target) {
        return;
    }
    if source == "尚未公布" && target != "-" {
        return;
    }
    *target = source.to_string();
}

/// 判斷日期欄是否已經是實際日期（而非 `-` 或「尚未公布」）。
fn is_actual_date(value: &str) -> bool {
    value.len() == 10 && value.split('-').count() == 3
}
