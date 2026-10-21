use super::*;

/// 日誌檔路徑帶有 `Rotate` 用來替換的日期樣板與 logger 名稱。
#[test]
fn log_path_carries_the_date_template_and_logger_name() {
    let path = Logger::get_log_path("default_info").expect("路徑應可建立");

    assert_eq!(path.parent(), Some(Path::new("log")));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some("%Y-%m-%d_default_info.log")
    );
}

/// 統計快照如實反映各計數器目前的值。
#[test]
fn writer_stats_snapshot_reports_current_counters() {
    let stats = LoggerWriterStats::new(LOG_CHANNEL_CAPACITY);
    stats.queued_messages.store(2, Ordering::Relaxed);
    stats.processed_messages.store(7, Ordering::Relaxed);
    stats.dropped_messages.store(1, Ordering::Relaxed);

    assert_eq!(
        stats.snapshot(),
        LoggerRuntimeStatus {
            queued_messages: 2,
            processed_messages: 7,
            dropped_messages: 1,
            channel_capacity: LOG_CHANNEL_CAPACITY,
        }
    );
}

/// 佇列已滿時訊息會被丟棄，並且不留下「排隊中」的假象。
///
/// 這裡直接建構 writer（不經 `Logger::new`），避免起背景 thread 真的寫檔。
#[tokio::test]
async fn send_drops_the_message_when_the_queue_is_full() {
    let (sender, _rx) = mpsc::channel::<String>(1);
    let writer = AsyncLogWriter {
        sender,
        stats: Arc::new(LoggerWriterStats::new(1)),
    };
    let logger = Logger {
        info_writer: writer.clone(),
        warn_writer: writer.clone(),
        error_writer: writer.clone(),
        debug_writer: writer.clone(),
    };

    // 容量 1：第一筆進佇列，第二筆無處可放只能丟棄。
    logger.send("first".to_string(), &writer);
    logger.send("second".to_string(), &writer);

    let snapshot = writer.diagnostics_snapshot();
    assert_eq!(snapshot.queued_messages, 1);
    assert_eq!(snapshot.dropped_messages, 1);
    assert_eq!(snapshot.channel_capacity, 1);
}

/// 接收端關閉後所有訊息都算丟棄，且不可 panic。
#[tokio::test]
async fn send_counts_drops_after_the_receiver_is_closed() {
    let (sender, rx) = mpsc::channel::<String>(4);
    drop(rx);
    let writer = AsyncLogWriter {
        sender,
        stats: Arc::new(LoggerWriterStats::new(4)),
    };
    let logger = Logger {
        info_writer: writer.clone(),
        warn_writer: writer.clone(),
        error_writer: writer.clone(),
        debug_writer: writer.clone(),
    };

    logger.error("boom".to_string());

    let snapshot = writer.diagnostics_snapshot();
    assert_eq!(snapshot.queued_messages, 0);
    assert_eq!(snapshot.dropped_messages, 1);
}

/// 四個等級的統計會彙總成單一摘要。
#[tokio::test]
async fn logger_snapshot_aggregates_every_level() {
    let make = || {
        let (sender, rx) = mpsc::channel::<String>(8);
        (
            AsyncLogWriter {
                sender,
                stats: Arc::new(LoggerWriterStats::new(8)),
            },
            rx,
        )
    };
    // receiver 必須留著：一旦 drop，channel 關閉會讓訊息全部改記為丟棄。
    let (info, _info_rx) = make();
    let (warn, _warn_rx) = make();
    let (error, _error_rx) = make();
    let (debug, _debug_rx) = make();
    let logger = Logger {
        info_writer: info,
        warn_writer: warn,
        error_writer: error,
        debug_writer: debug,
    };

    logger.info("i");
    logger.warn("w");
    logger.error("e");
    logger.debug("d");

    let snapshot = logger.diagnostics_snapshot();
    assert_eq!(snapshot.queued_messages, 4);
    assert_eq!(snapshot.dropped_messages, 0);
    assert_eq!(snapshot.channel_capacity, 32);
}
