//! Handlers module — Axum HTTP request handlers for the ingestion Lambda.
//!
//! Each sub-module owns one API resource:
//!
//! | Module      | Endpoint                  | Description                        |
//! |-------------|---------------------------|------------------------------------|
//! | [`webhook`] | `POST /webhooks/receive`  | Ingest and enqueue a webhook event |

pub mod webhook;
