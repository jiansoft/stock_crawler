//! 主機健康檢查的解析與格式化：`get_throttled`／`vcgencmd`、`/proc/meminfo` 與告警文字用的單位換算。

use super::*;

/// 解析 `get_throttled`（十六進位，可帶 `0x`）；格式不符回 `None`。
pub(crate) fn parse_throttled(text: &str) -> Option<u32> {
    let text = text.trim();
    let digits = text
        .strip_prefix("0x")
        .or_else(|| text.strip_prefix("0X"))
        .unwrap_or(text);
    u32::from_str_radix(digits, 16).ok()
}

/// 解析 `vcgencmd get_throttled` 的輸出（`throttled=0x70000`）；格式不符回 `None`。
pub(crate) fn parse_vcgencmd_throttled(text: &str) -> Option<u32> {
    parse_throttled(text.trim().strip_prefix("throttled=")?)
}

/// 解析 `/proc/meminfo`；缺 `MemAvailable` 回 `None`，沒有 swap 欄位視為沒有 swap。
pub(crate) fn parse_meminfo(text: &str) -> Option<MemInfo> {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            rest.split_whitespace().next()?.parse::<u64>().ok()
        })
    };
    Some(MemInfo {
        available_kib: field("MemAvailable")?,
        swap_total_kib: field("SwapTotal").unwrap_or(0),
        swap_free_kib: field("SwapFree").unwrap_or(0),
    })
}

/// `get_throttled` 目前位元的中文說明，依位元順序。
pub(super) fn throttled_labels(flags: u32) -> Vec<&'static str> {
    [
        (UNDER_VOLTAGE_NOW, "供電不足"),
        (FREQ_CAPPED_NOW, "頻率受限"),
        (THROTTLED_NOW, "降頻"),
        (SOFT_TEMP_LIMIT_NOW, "溫度達軟上限"),
    ]
    .into_iter()
    .filter(|(bit, _)| flags & bit != 0)
    .map(|(_, label)| label)
    .collect()
}

/// 開機以來位元（已右移到 bit 0～3）的中文說明，例如「曾供電不足、曾降頻」；沒有任何位元時為「無」。
pub(super) fn since_boot_text(flags: u32) -> String {
    if flags == 0 {
        return "無".to_owned();
    }
    throttled_labels(flags)
        .into_iter()
        .map(|label| format!("曾{label}"))
        .collect::<Vec<_>>()
        .join("、")
}

/// 取出 `vcgencmd` 值中的開機以來位元，右移到 bit 0～3。
pub(super) fn since_boot_flags(value: u32) -> u32 {
    (value >> SINCE_BOOT_SHIFT) & 0xF
}

/// KiB 換成 MB（取整）。
pub(super) fn mb(kib: u64) -> u64 {
    kib / 1024
}

/// 秒數文字（一位小數）。
pub(super) fn seconds(duration: Duration) -> String {
    format!("{:.1}", duration.as_secs_f64())
}
