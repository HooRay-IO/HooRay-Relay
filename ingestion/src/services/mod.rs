//! Services module — business logic layer for the ingestion Lambda.
//!
//! Each sub-module owns one discrete responsibility:
//!
//! | Module          | Responsibility                                          |
//! |-----------------|--------------------------------------------------------|
//! | [`dynamodb`]    | AWS DynamoDB client factory and [`AppConfig`] struct   |
//! | [`idempotency`] | Conditional DynamoDB write to deduplicate inbound requests |
//! | [`events`]      | Persist new webhook events with 30-day TTL             |
//! | [`queue`]       | Enqueue events onto SQS with `customer_id` message attribute |
//! | [`configs`]     | DynamoDB CRUD for customer delivery configurations     |

pub mod configs;
pub mod dynamodb;
pub mod events;
pub mod idempotency;
pub mod queue;
pub mod reconcile;
