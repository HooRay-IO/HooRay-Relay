//! Handlers module — Axum HTTP request handlers for the ingestion Lambda.
//!
//! Each sub-module owns one API resource:
//!
//! | Module      | Endpoint                              | Description                        |
//! |-------------|---------------------------------------|------------------------------------|
//! | [`webhook`] | `POST /webhooks/receive`              | Ingest and enqueue a webhook event |
//! | [`config`]  | `POST /webhooks/configs`              | Register a customer delivery config |
//! | [`config`]  | `GET  /webhooks/configs?customer_id=` | Retrieve a customer delivery config |

pub mod config;
pub mod webhook;
