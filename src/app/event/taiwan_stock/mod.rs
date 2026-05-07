/// 財務年報
pub mod annual_eps;
/// 收盤事件
pub mod closing;
/// 除息日的事件
pub mod ex_dividend;
/// 股利發放日的事件
pub mod payable_date;
/// 公開申購公告
pub mod public;
/// 財務季報
pub mod quarter_eps;

use rust_decimal::Decimal;

/// 將內部 `member_id` 轉成訊息內可讀的成員名稱。
pub(crate) fn member_label(member_id: i64) -> String {
    match member_id {
        1 => "Eddie".to_string(),
        2 => "Unice".to_string(),
        3 => "Hugo".to_string(),
        4 => "Aiden".to_string(),
        _ => format!("Member {}", member_id),
    }
}

/// 將數字字串的整數部分補上千位分隔符。
pub(crate) fn add_thousand_separators(raw: &str) -> String {
    let (sign, digits) = if let Some(rest) = raw.strip_prefix('-') {
        ("-", rest)
    } else {
        ("", raw)
    };
    let mut result = String::with_capacity(raw.len() + raw.len() / 3);

    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }

    let formatted: String = result.chars().rev().collect();
    format!("{sign}{formatted}")
}

/// 將 `Decimal` 格式化成帶千位分隔符的字串，並保留最多兩位小數。
pub(crate) fn format_decimal_with_commas(value: Decimal) -> String {
    let normalized = value.round_dp(2).normalize().to_string();
    let (integer_part, fractional_part) = normalized
        .split_once('.')
        .map_or((normalized.as_str(), None), |(int_part, frac_part)| {
            (int_part, Some(frac_part))
        });
    let formatted_integer = add_thousand_separators(integer_part);

    match fractional_part {
        Some(frac_part) => format!("{formatted_integer}.{frac_part}"),
        None => formatted_integer,
    }
}

/// 將 `Decimal` 格式化成帶千位分隔符的字串，並固定兩位小數。
pub(crate) fn format_decimal_with_fixed_two_commas(value: Decimal) -> String {
    let rounded = value.round_dp(2).to_string();
    let (integer_part, fractional_part) = rounded
        .split_once('.')
        .map_or((rounded.as_str(), ""), |(int_part, frac_part)| {
            (int_part, frac_part)
        });
    let formatted_integer = add_thousand_separators(integer_part);

    match fractional_part.len() {
        0 => format!("{formatted_integer}.00"),
        1 => format!("{formatted_integer}.{fractional_part}0"),
        _ => format!("{formatted_integer}.{}", &fractional_part[..2]),
    }
}

/// 將持股股數格式化成帶千位分隔符的字串。
pub(crate) fn format_share_quantity(value: i64) -> String {
    add_thousand_separators(&value.to_string())
}
