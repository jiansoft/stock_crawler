# ADR-001：Phase 1 暫不拆 Cargo Workspace

日期：2026-06-13

## 狀態

Accepted for Phase 1

## 背景

本專案目前是單一 crate `stock_crawler`，但程式碼已依 `core`、`domain`、`app`、`infra`、`interfaces` 建立初步分層。使用者要求行為等價重構，且在完成分析與重構計畫前不得修改程式碼。

## 決策

Phase 1 與 Phase 2 不拆 Cargo workspace，不搬移 crate 邊界。先在現有單 crate 內建立測試保護、依賴方向盤點與模組拆分策略。

## 理由

- 拆 workspace 會同時影響 module path、visibility、build.rs generated code、CI cache、Docker/build scripts，風險高。
- 目前已有分層目錄，可先在不改 public API 的前提下改善內部耦合。
- 行為等價重構優先順序應是測試保護與小步重構，不是一開始做大規模搬移。

## 後果

- 短期仍需接受單 crate 中跨層引用較容易發生。
- 後續可用 lint、文件與 code review 約束依賴方向。
- 若未來要拆 workspace，需另開 ADR，並先完成 generated gRPC、domain error、repository trait 邊界穩定化。

