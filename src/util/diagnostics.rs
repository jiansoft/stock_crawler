//! 執行期 diagnostics 輔助工具。

/// 低成本的背景任務執行狀態快照。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TaskRuntimeStatus {
    /// 任務開關是否仍處於啟用狀態。
    pub enabled: bool,
    /// 目前仍在執行中的 task 數量。
    pub active_tasks: usize,
    /// 最近一次啟動 task 的世代編號。
    pub last_generation: u64,
}

impl TaskRuntimeStatus {
    /// 建立新的 task 狀態快照。
    pub const fn new(enabled: bool, active_tasks: usize, last_generation: u64) -> Self {
        Self {
            enabled,
            active_tasks,
            last_generation,
        }
    }
}

/// 目前程序的記憶體用量快照。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProcessMemoryStats {
    /// 常駐記憶體大小，單位 KiB。
    pub vm_rss_kib: u64,
    /// 虛擬記憶體大小，單位 KiB。
    pub vm_size_kib: u64,
}

/// 長時間執行程序的 allocator 調校結果。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AllocatorTuningResult {
    /// 是否至少套用了一項 glibc mallopt 設定。
    pub applied: bool,
    /// 是否成功限制 arena 數量。
    pub arena_max_applied: bool,
    /// 是否成功調低 trim threshold。
    pub trim_threshold_applied: bool,
}

/// 嘗試讀取目前程序的記憶體用量。
///
/// Linux 會讀取 `/proc/self/status` 內的 `VmRSS` / `VmSize`；
/// 其他平台回傳 `None`。
pub fn read_process_memory_stats() -> Option<ProcessMemoryStats> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let mut stats = ProcessMemoryStats::default();

        for line in status.lines() {
            if let Some(value) = line.strip_prefix("VmRSS:") {
                stats.vm_rss_kib = parse_status_kib(value)?;
                continue;
            }

            if let Some(value) = line.strip_prefix("VmSize:") {
                stats.vm_size_kib = parse_status_kib(value)?;
            }
        }

        return Some(stats);
    }

    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// 嘗試要求 allocator 將可回收的頁面歸還給作業系統。
///
/// 目前只在 Linux GNU 環境呼叫 `malloc_trim(0)`；
/// 其他平台或目標環境回傳 `false`。
pub fn trim_allocator_memory() -> bool {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        return malloc_trim(0) != 0;
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        false
    }
}

/// 在程序啟動初期調校 glibc allocator，降低長時間服務的 RSS 工作集。
///
/// 目前只在 Linux GNU 環境透過 `mallopt`：
/// - 將 `M_ARENA_MAX` 壓到 2，避免 arena 數量隨執行緒與高峰配置膨脹。
/// - 將 `M_TRIM_THRESHOLD` 調低到 128KiB，讓 allocator 較願意歸還空頁。
pub fn tune_allocator_for_long_running_process() -> AllocatorTuningResult {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        let arena_max_applied = mallopt(M_ARENA_MAX, 2) != 0;
        let trim_threshold_applied = mallopt(M_TRIM_THRESHOLD, 128 * 1024) != 0;

        return AllocatorTuningResult {
            applied: arena_max_applied || trim_threshold_applied,
            arena_max_applied,
            trim_threshold_applied,
        };
    }

    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    {
        AllocatorTuningResult::default()
    }
}

#[cfg(target_os = "linux")]
fn parse_status_kib(raw: &str) -> Option<u64> {
    raw.split_whitespace().next()?.parse().ok()
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
unsafe extern "C" {
    fn malloc_trim(pad: usize) -> i32;
    fn mallopt(param: i32, value: i32) -> i32;
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_TRIM_THRESHOLD: i32 = -1;
#[cfg(all(target_os = "linux", target_env = "gnu"))]
const M_ARENA_MAX: i32 = -8;
