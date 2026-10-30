//! musl 目標的全域記憶體分配器（mimalloc）與執行期回收旗標；其他目標此模組為空。

// ── 條件編譯引用（musl + mimalloc）─────────────────────────────────────────
// musl 是 Linux 上的靜態 C 函式庫（相對於 glibc 的動態版本）。
// 使用 musl 目標編譯出的執行檔不依賴系統 glibc，非常適合放進 Alpine Linux 容器，
// 可大幅縮小映像體積（scratch / alpine 基底映像）。
// glibc 內建的 ptmalloc 在 musl 環境下效能不佳；mimalloc 是 Microsoft 開源的
// 高效能記憶體分配器，可大幅改善小物件頻繁分配的場景（例如爬蟲 HTTP 回應解析）。
#[cfg(all(target_os = "linux", target_env = "musl"))]
use mimalloc::MiMalloc;
// `c_char` 用於與 C 的 setenv() 互動，傳遞 null-terminated 字串指標。
#[cfg(all(target_os = "linux", target_env = "musl"))]
use std::ffi::c_char;

// ── 全域記憶體分配器（musl 目標）────────────────────────────────────────────
// `#[global_allocator]` 告訴 Rust 編譯器：用 MiMalloc 取代系統預設的 malloc/free。
// 只在 musl 目標啟用；glibc 目標使用系統 malloc（glibc 2.17+ 的 ptmalloc 已夠好）。
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[global_allocator]
static GLOBAL_ALLOCATOR: MiMalloc = MiMalloc;

/// 在 `main` 之前為 musl + mimalloc 設定較積極的回收策略。
///
/// 這些選項來自 mimalloc 官方環境參數：
/// - `MIMALLOC_PURGE_DELAY=0`：頁面一旦空出就立即 purge。
/// - `MIMALLOC_PURGE_DECOMMITS=1`：purge 時用 decommit / `MADV_DONTNEED`，讓 RSS 較快下降。
/// - `MIMALLOC_ALLOW_THP=0`：避免 THP 讓 purge 顆粒變粗，影響回收速度。
/// - `MIMALLOC_ALLOW_LARGE_OS_PAGES=0`：避免不可 purge 的 large OS pages。
/// - `MIMALLOC_ARENA_EAGER_COMMIT=0`：避免過早 commit 大 arena。
#[cfg(all(target_os = "linux", target_env = "musl"))]
#[used]
// `.init_array` 是 ELF 二進位格式的初始化函式指標陣列。
// 動態連結器在執行 main() 之前會逐一呼叫陣列中的函式指標，
// 確保環境變數在 mimalloc 第一次分配記憶體前就已設好。
// `overwrite = 0` 表示若環境中已有該變數則不覆蓋，方便 Docker 從外部覆寫行為。
#[unsafe(link_section = ".init_array")]
static INIT_MIMALLOC_ENV: extern "C" fn() = init_mimalloc_env;

/// 透過 libc `setenv` 設定 mimalloc 執行期旗標。
///
/// 之所以用 `extern "C" fn` 而非 `#[ctor]` crate，是為了盡量減少外部依賴，
/// 並維持對 `.init_array` 機制的完整控制。
#[cfg(all(target_os = "linux", target_env = "musl"))]
extern "C" fn init_mimalloc_env() {
    // Safety: 所有字串字面值均為 null-terminated（\0 結尾）且具靜態生命週期，
    // 指標永遠有效；overwrite=0 保證不覆蓋外部設定。
    unsafe {
        setenv(
            b"MIMALLOC_PURGE_DELAY\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_PURGE_DECOMMITS\0".as_ptr().cast::<c_char>(),
            b"1\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ALLOW_THP\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ALLOW_LARGE_OS_PAGES\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
        setenv(
            b"MIMALLOC_ARENA_EAGER_COMMIT\0".as_ptr().cast::<c_char>(),
            b"0\0".as_ptr().cast::<c_char>(),
            0,
        );
    }
}

// libc setenv 的 Rust FFI 宣告。
// `unsafe extern "C"` 表示這是遵循 C ABI 的外部函式，
// 呼叫者必須自行確保參數合法（指標不為 null、字串以 \0 結尾）。
#[cfg(all(target_os = "linux", target_env = "musl"))]
unsafe extern "C" {
    fn setenv(name: *const c_char, value: *const c_char, overwrite: i32) -> i32;
}
