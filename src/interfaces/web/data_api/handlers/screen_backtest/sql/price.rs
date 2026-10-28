//! 回測 SQL 的報酬算式：同天期報酬與次日開盤版的開盤價／收盤價比。

/// 同天期報酬（`f` 為個股、`e` 為 0050 的 `daily_forward_return`）。
macro_rules! horizon_return {
    ($alias:literal) => {
        concat!(
            "CASE $4::int WHEN 5 THEN ",
            $alias,
            ".return_5d WHEN 20 THEN ",
            $alias,
            ".return_20d WHEN 60 THEN ",
            $alias,
            ".return_60d WHEN 120 THEN ",
            $alias,
            ".return_120d ELSE ",
            $alias,
            ".return_240d END"
        )
    };
}

/// 次日開盤版：收盤報酬乘上出場日、進場日的開盤價／收盤價比（`$exit`／`$entry` 為該日日線的別名）。
macro_rules! open_to_open {
    ($alias:literal, $entry:literal, $exit:literal) => {
        concat!(
            "((1 + (",
            horizon_return!($alias),
            ") / 100.0) * (CASE WHEN ",
            $exit,
            ".\"OpeningPrice\" > 0 AND ",
            $exit,
            ".\"ClosingPrice\" > 0 THEN ",
            $exit,
            ".\"OpeningPrice\" / ",
            $exit,
            ".\"ClosingPrice\" ELSE 1 END) / (CASE WHEN ",
            $entry,
            ".\"OpeningPrice\" > 0 AND ",
            $entry,
            ".\"ClosingPrice\" > 0 THEN ",
            $entry,
            ".\"OpeningPrice\" / ",
            $entry,
            ".\"ClosingPrice\" ELSE 1 END) - 1) * 100"
        )
    };
}

pub(super) use {horizon_return, open_to_open};
