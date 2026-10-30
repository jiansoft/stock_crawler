//! 執行檔的啟動與關機流程。
//!
//! 只屬於 bin target（`src/main.rs`），CI 的測試不會執行，因此與 `main.rs` 一起列在
//! `codecov.yml` 的 ignore。`main` 依序呼叫：[`telemetry::init`] → [`wiring::register_adapters`]
//! → [`startup::start`] → [`shutdown::wait_and_stop`]。

mod allocator;
pub(crate) mod shutdown;
pub(crate) mod startup;
pub(crate) mod telemetry;
pub(crate) mod wiring;
