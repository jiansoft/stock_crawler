use chrono::TimeDelta;

use crate::core::logging;

use super::*;

/// 驗證基本寫檔流程。
#[tokio::test]
#[ignore]
async fn test_basic_write() {
    dotenvy::dotenv().ok();
    logging::debug_file_async("開始 test_basic_write".to_string());

    let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
    let now = Local::now();

    // 使用新的 write_msg 方法
    let msg = format!("{} 測試訊息\r\n", now.format("%F %X%.6f"));
    if let Err(why) = r.write_msg(now, msg.as_bytes()) {
        logging::error_console(format!("Failed to write: {:?}", why));
    }

    logging::debug_file_async("結束 test_basic_write".to_string());
}

/// 驗證跨日期時會切換新檔案。
#[tokio::test]
#[ignore]
async fn test_date_rotation() {
    dotenvy::dotenv().ok();
    logging::debug_file_async("開始 test_date_rotation".to_string());

    let mut r = Rotate::new("log/%Y-%m-%d-test.log".to_string());
    let mut now = Local::now();

    // 第一天寫入
    let msg1 = format!("{} Day 1\r\n", now.format("%F %X%.6f"));
    let _ = r.write_msg(now, msg1.as_bytes());

    // 模擬下一天
    now += TimeDelta::try_days(1).unwrap();
    let msg2 = format!("{} Day 2\r\n", now.format("%F %X%.6f"));
    let _ = r.write_msg(now, msg2.as_bytes());

    logging::debug_file_async("結束 test_date_rotation".to_string());
}

/// 驗證超過檔案大小限制時會輪轉。
#[tokio::test]
#[ignore]
async fn test_size_rotation() {
    dotenvy::dotenv().ok();
    logging::debug_file_async("開始 test_size_rotation".to_string());

    // 設定很小的檔案大小限制 (1KB) 來測試輪轉
    let mut r = Rotate::with_options(
        "log/%Y-%m-%d-size-test.log".to_string(),
        1024, // 1 KB
        7,    // 保留 7 天
    );

    let now = Local::now();

    // 寫入超過 1KB 的資料，觸發輪轉
    for i in 0..20 {
        let msg = format!(
            "{} Line {} - {}\r\n",
            now.format("%F %X%.6f"),
            i,
            "X".repeat(100)
        );
        if let Err(why) = r.write_msg(now, msg.as_bytes()) {
            logging::error_console(format!("Failed to write line {}: {:?}", i, why));
        }
    }

    logging::debug_file_async(format!(
        "最終 generation: {}, current_size: {}",
        r.generation, r.current_size
    ));
    logging::debug_file_async("結束 test_size_rotation".to_string());
}

/// 驗證外部設定的套用、範圍收斂，以及 `Rotate::new` 會跟隨全域設定。
///
/// 測試結束時還原預設值，避免影響同一 process 內的其他測試。
#[test]
fn test_configure_limits() {
    // 0 代表未設定，維持既有值
    configure(0, 0);
    assert_eq!(
        effective_settings(),
        (DEFAULT_MAX_SIZE, DEFAULT_MAX_AGE_DAYS)
    );

    // 一般設定值直接生效
    configure(20, 14);
    assert_eq!(effective_settings(), (20 * 1024 * 1024, 14));

    // 已建立的 Rotate（跟隨全域）也會讀到新值
    let follower = Rotate::new("log/%Y-%m-%d-configure-test.log".to_string());
    assert_eq!(follower.max_size(), 20 * 1024 * 1024);
    assert_eq!(follower.max_age(), TimeDelta::days(14));

    // 以固定值建立者不受全域影響
    let fixed = Rotate::with_options("log/%Y-%m-%d-configure-test.log".to_string(), 4096, 3);
    assert_eq!(fixed.max_size(), 4096);
    assert_eq!(fixed.max_age(), TimeDelta::days(3));

    // 超出範圍者收斂至邊界
    configure(4096, 3650);
    assert_eq!(
        effective_settings(),
        (MAX_CONFIGURABLE_SIZE, MAX_CONFIGURABLE_AGE_DAYS)
    );

    // 還原預設值
    GLOBAL_MAX_SIZE.store(DEFAULT_MAX_SIZE, Ordering::Relaxed);
    GLOBAL_MAX_AGE_DAYS.store(DEFAULT_MAX_AGE_DAYS, Ordering::Relaxed);
}

/// 驗證封存檔名以「最後一筆日誌時間」標記，並在同秒撞名時遞增序號。
#[test]
fn test_timestamp_filename() {
    let r = Rotate::new("log/%Y-%m-%d-app.log".to_string());
    let base = "log/2025-02-03-app.log";
    let sep = std::path::MAIN_SEPARATOR;

    assert_eq!(
        r.generate_full_fn(base, "00-05-32", 0),
        format!("log{sep}2025-02-03-app.00-05-32.log")
    );
    assert_eq!(
        r.generate_full_fn(base, "01-03-07", 0),
        format!("log{sep}2025-02-03-app.01-03-07.log")
    );
    assert_eq!(
        r.generate_full_fn(base, "01-03-07", 2),
        format!("log{sep}2025-02-03-app.01-03-07-2.log")
    );
}

/// 建立本次測試專用的暫存目錄，避免與其他測試（或正式 log 目錄）互相干擾。
fn temp_log_dir(tag: &str) -> PathBuf {
    let unique = format!(
        "{}-{}-{}",
        tag,
        std::process::id(),
        Local::now().timestamp_nanos_opt().unwrap_or_default()
    );
    let dir = std::env::temp_dir()
        .join("stock_rust-rotate-test")
        .join(unique);
    fs::create_dir_all(&dir).expect("建立暫存日誌目錄失敗");

    dir
}

/// 列出目錄內所有檔名（排序後回傳）。
fn file_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .expect("讀取暫存日誌目錄失敗")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .collect();
    names.sort();

    names
}

/// 驗證輪轉命名策略：
/// 1. 正在寫入的檔案固定為不含時間戳的檔名；
/// 2. 超過大小上限時，舊檔被更名為帶時間戳的封存檔，彼此不覆蓋。
#[test]
fn test_active_file_has_no_timestamp() {
    let dir = temp_log_dir("active");
    let pattern = dir.join("%Y-%m-%d-app.log").to_string_lossy().to_string();

    // 512 bytes 上限，強制多次輪轉
    let mut r = Rotate::with_options(pattern, 512, 7);
    let now = Local::now();

    for i in 0..50 {
        let msg = format!("Line {:03} - {}\r\n", i, "X".repeat(50));
        r.write_msg(now, msg.as_bytes()).unwrap();
    }

    let final_generation = r.generation;
    assert!(
        final_generation >= 3,
        "應該至少輪轉 3 次，實際: {}",
        final_generation
    );

    let base_name = now.format("%Y-%m-%d-app").to_string();
    let stamp = now.format("%H-%M-%S").to_string();
    let files = file_names(&dir);

    // 活躍檔（不含時間戳）必定存在，且是目前寫入的目標
    let active_name = format!("{}.log", base_name);
    assert!(
        files.contains(&active_name),
        "缺少活躍檔: {}。實際檔案: {:?}",
        active_name,
        files
    );
    assert!(r.cur_fn.ends_with(&active_name), "cur_fn 應指向活躍檔");

    // 封存檔數量 = 輪轉次數；本測試所有寫入共用同一個 now，時間戳相同、以序號區分
    assert_eq!(
        files.len() as u32,
        final_generation + 1,
        "檔案數量應為 輪轉次數+1（活躍檔），實際檔案: {:?}",
        files
    );
    for seq in 0..final_generation {
        let expected_name = if seq == 0 {
            format!("{}.{}.log", base_name, stamp)
        } else {
            format!("{}.{}-{}.log", base_name, stamp, seq)
        };
        assert!(
            files.contains(&expected_name),
            "缺少封存檔: {}。實際檔案: {:?}",
            expected_name,
            files
        );
    }

    drop(r);
    let _ = fs::remove_dir_all(&dir);
}

/// 驗證封存檔的時間戳取自該檔「最後一筆日誌」的時間，而非輪轉當下的時間。
#[test]
fn test_archive_stamp_uses_last_write_time() {
    let dir = temp_log_dir("stamp");
    let pattern = dir.join("%Y-%m-%d-app.log").to_string_lossy().to_string();

    let mut r = Rotate::with_options(pattern, 512, 7);
    // 基準時間釘在當天中午，而不是 Local::now()：這個測試會把時間往後推一小時
    // 來觸發「大小輪轉」，若在 23 點之後執行，加一小時就跨日了，
    // write_msg 會改走跨日切檔的路徑並把 generation 歸零，測試每天必失敗一小時。
    let first = Local::now()
        .date_naive()
        .and_hms_opt(12, 0, 0)
        .and_then(|naive| naive.and_local_timezone(Local).earliest())
        .expect("測試基準時間應合法");
    // 寫到接近上限（每行 63 bytes × 8 = 504 < 512），輪轉留給下一次寫入觸發
    for i in 0..8 {
        let msg = format!("Line {:03} - {}\r\n", i, "X".repeat(50));
        r.write_msg(first, msg.as_bytes()).unwrap();
    }
    assert_eq!(r.generation, 0, "第一個檔案尚未輪轉");

    // 一小時後再寫入，觸發輪轉；封存檔名應標記 first（最後一筆日誌時間）
    let later = first + TimeDelta::try_hours(1).unwrap();
    r.write_msg(later, b"after rotate\r\n").unwrap();
    assert_eq!(r.generation, 1, "應已輪轉一次");

    let base_name = first.format("%Y-%m-%d-app").to_string();
    let files = file_names(&dir);
    let expected_archive = format!("{}.{}.log", base_name, first.format("%H-%M-%S"));
    let unexpected_archive = format!("{}.{}.log", base_name, later.format("%H-%M-%S"));

    assert!(
        files.contains(&expected_archive),
        "封存檔應標記最後一筆日誌時間 {}。實際檔案: {:?}",
        expected_archive,
        files
    );
    assert!(
        !files.contains(&unexpected_archive),
        "封存檔不應使用輪轉當下時間 {}。實際檔案: {:?}",
        unexpected_archive,
        files
    );

    drop(r);
    let _ = fs::remove_dir_all(&dir);
}

/// 驗證重啟後會接續寫入同一天的活躍檔，且既有大小納入輪轉判斷。
#[test]
fn test_restart_appends_to_active_file() {
    let dir = temp_log_dir("restart");
    let pattern = dir.join("%Y-%m-%d-app.log").to_string_lossy().to_string();
    let now = Local::now();

    let mut first = Rotate::with_options(pattern.clone(), 1024, 7);
    first.write_msg(now, b"first run\r\n").unwrap();
    let size_after_first = first.current_size;
    drop(first);

    let mut second = Rotate::with_options(pattern, 1024, 7);
    second.write_msg(now, b"second run\r\n").unwrap();

    assert_eq!(second.generation, 0, "重啟不應產生輪轉");
    assert!(
        second.current_size > size_after_first,
        "重啟後應接續既有檔案大小"
    );
    assert_eq!(file_names(&dir).len(), 1, "重啟後不應產生額外檔案");

    let content = fs::read_to_string(&second.cur_fn).unwrap();
    assert!(content.contains("first run") && content.contains("second run"));

    drop(second);
    let _ = fs::remove_dir_all(&dir);
}
