//! 籌碼資料（三大法人買賣超、融資融券餘額）的共用 DTO 與欄位解析。

use std::collections::HashMap;

/// 單一股票單日的三大法人買賣超（單位：股，正值為買超）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InstitutionalFlow {
    /// 外資及陸資（含外資自營商）。
    pub foreign: i64,
    /// 投信。
    pub trust: i64,
    /// 自營商（自行買賣與避險合計）。
    pub dealer: i64,
}

impl InstitutionalFlow {
    /// 三大法人合計。
    pub fn total(&self) -> i64 {
        self.foreign + self.trust + self.dealer
    }
}

/// 單一股票單日的融資融券餘額（單位：張）。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarginBalance {
    /// 前日融資餘額。
    pub margin_previous: i64,
    /// 今日融資餘額。
    pub margin_today: i64,
    /// 前日融券餘額。
    pub short_previous: i64,
    /// 今日融券餘額。
    pub short_today: i64,
}

/// 解析交易所的整數欄位（去掉千分位與空白；`+`、`-` 符號保留）。
pub(crate) fn parse_count(raw: &str) -> Option<i64> {
    let cleaned: String = raw
        .chars()
        .filter(|c| !matches!(c, ',' | ' ' | '\u{a0}'))
        .collect();
    cleaned.parse().ok()
}

/// 依欄位位置把資料列整理成 `代號 → 值`；欄位不足或數字無法解析的列略過。
pub(crate) fn collect_rows<T>(
    rows: &[Vec<String>],
    parse: impl Fn(&[String]) -> Option<T>,
) -> HashMap<String, T> {
    rows.iter()
        .filter_map(|row| {
            let symbol = row.first()?.trim();
            if symbol.is_empty() {
                return None;
            }
            Some((symbol.to_string(), parse(row)?))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_count_handles_separators_and_signs() {
        assert_eq!(parse_count("9,770,520"), Some(9_770_520));
        assert_eq!(parse_count("-2,851,652"), Some(-2_851_652));
        assert_eq!(parse_count(" 30,135 "), Some(30_135));
        assert_eq!(parse_count("--"), None);
        assert_eq!(parse_count(""), None);
    }

    #[test]
    fn institutional_flow_total_sums_three_parties() {
        let flow = InstitutionalFlow {
            foreign: 9_770_520,
            trust: 589_108,
            dealer: 491_655,
        };
        assert_eq!(flow.total(), 10_851_283);
    }

    #[test]
    fn collect_rows_skips_blank_symbols_and_unparsable_rows() {
        let rows = vec![
            vec!["2330".to_string(), "1".to_string()],
            vec![" ".to_string(), "2".to_string()],
            vec!["2317".to_string(), "x".to_string()],
        ];
        let map = collect_rows(&rows, |row| parse_count(&row[1]));
        assert_eq!(map.len(), 1);
        assert_eq!(map["2330"], 1);
    }
}
