//! Services module — business logic layer for the ingestion Lambda.
//!
//! Each sub-module owns one discrete responsibility:
//!
//! | Module          | Responsibility                                          |
//! |-----------------|--------------------------------------------------------|
//! | [`idempotency`] | Conditional DynamoDB write to deduplicate inbound requests |
//! | [`events`]      | Persist new webhook events with 30-day TTL             |

pub mod events;
pub mod idempotency;
