//! 版本化唯讀股票 Data API 與其線上 OpenAPI 文件。
//!
//! 此模組把 PostgreSQL 的內部 schema 隔離於 HTTP 契約之外；MCP 等內網服務
//! 只需依 `/api-docs/openapi.json` 建立 client，並可從 `/swagger-ui` 互動測試。
//!
//! 檔案分工：`routes` 負責路由註冊與 Bearer 保護範圍、`openapi` 負責 OpenAPI
//! 文件定義、`handlers` 是各 endpoint 的實作、`dto` 是 HTTP 契約型別、
//! `auth` 是 Bearer token 驗證 middleware。

mod auth;
mod dto;
mod handlers;
mod openapi;
mod routes;

pub(super) use routes::router;
