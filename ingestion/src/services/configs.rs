//! Config service — DynamoDB CRUD for the `webhook_configs` table.
//!
//! Each customer has exactly one config record:
//!
//! ```text
//! PK = CUSTOMER#{customer_id}
//! SK = CONFIG
//! ```
//!
//! The record contains the delivery URL, HMAC signing secret, retry limit, and
//! active flag.  The worker reads this before each delivery attempt.

use aws_sdk_dynamodb::Client as DynamoClient;
use aws_sdk_dynamodb::types::AttributeValue;
use serde_dynamo::aws_sdk_dynamodb_1::{from_item, to_item};
use tracing::{debug, info};

use crate::model::{IngestionError, WebhookConfig};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Persist a [`WebhookConfig`] to DynamoDB (upsert — last write wins).
///
/// Uses an unconditional `PutItem` so callers can rotate secrets or update the
/// delivery URL by posting again.
///
/// # Errors
///
/// Returns [`IngestionError::DynamoDb`] on any AWS SDK error, or
/// [`IngestionError::Serialization`] if `serde_dynamo` fails to encode the item.
pub async fn put_config(
    client: &DynamoClient,
    table: &str,
    config: &WebhookConfig,
) -> Result<(), IngestionError> {
    debug!(
        customer_id = %config.customer_id,
        table,
        "serializing config for DynamoDB"
    );

    // Serialize to DynamoDB attribute map.
    let mut item = to_item(config).map_err(|e| IngestionError::Serialization(e.to_string()))?;

    // Inject the DynamoDB PK / SK (not stored on the struct itself).
    item.insert("pk".to_string(), AttributeValue::S(config.pk()));
    item.insert(
        "sk".to_string(),
        AttributeValue::S(WebhookConfig::sk().to_string()),
    );

    client
        .put_item()
        .table_name(table)
        .set_item(Some(item))
        .send()
        .await
        .map_err(|e| IngestionError::DynamoDb(e.to_string()))?;

    info!(
        customer_id = %config.customer_id,
        table,
        "webhook config persisted"
    );

    Ok(())
}

/// Fetch a [`WebhookConfig`] from DynamoDB by `customer_id`.
///
/// # Errors
///
/// - [`IngestionError::ItemNotFound`] — no config registered for this customer
/// - [`IngestionError::DynamoDb`] — AWS SDK error
/// - [`IngestionError::DecodeDynamo`] — `serde_dynamo` decode failure
pub async fn fetch_config(
    client: &DynamoClient,
    table: &str,
    customer_id: &str,
) -> Result<WebhookConfig, IngestionError> {
    let pk = format!("CUSTOMER#{customer_id}");
    let sk = WebhookConfig::sk();

    debug!(customer_id, table, "fetching webhook config from DynamoDB");

    let resp = client
        .get_item()
        .table_name(table)
        .key("pk", AttributeValue::S(pk.clone()))
        .key("sk", AttributeValue::S(sk.to_string()))
        .send()
        .await
        .map_err(|e| IngestionError::DynamoDb(e.to_string()))?;

    let item = resp.item.ok_or_else(|| IngestionError::ItemNotFound {
        entity: "WebhookConfig",
        key: pk.clone(),
    })?;

    let config: WebhookConfig = from_item(item)?;

    debug!(customer_id, "webhook config retrieved");

    Ok(config)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use crate::model::WebhookConfig;

    fn sample_config() -> WebhookConfig {
        WebhookConfig {
            customer_id: "cust_xyz123".to_string(),
            url: "https://customer.example.com/webhooks".to_string(),
            secret: "whsec_a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6".to_string(),
            max_retries: 3,
            active: true,
            created_at: 1_707_840_000,
            updated_at: 1_707_840_000,
        }
    }

    // -----------------------------------------------------------------------
    // DynamoDB key contract
    // -----------------------------------------------------------------------

    #[test]
    fn config_pk_matches_dynamodb_contract() {
        let config = sample_config();
        assert_eq!(config.pk(), "CUSTOMER#cust_xyz123");
    }

    #[test]
    fn config_sk_matches_dynamodb_contract() {
        assert_eq!(WebhookConfig::sk(), "CONFIG");
    }

    // -----------------------------------------------------------------------
    // serde_dynamo round-trip (no live DynamoDB needed)
    // -----------------------------------------------------------------------

    #[test]
    fn config_serializes_and_deserializes_round_trip() {
        use serde_dynamo::aws_sdk_dynamodb_1::{from_item, to_item};

        let original = sample_config();
        let item = to_item(&original).expect("serialization should succeed");
        let decoded: WebhookConfig = from_item(item).expect("deserialization should succeed");

        assert_eq!(decoded.customer_id, original.customer_id);
        assert_eq!(decoded.url, original.url);
        assert_eq!(decoded.secret, original.secret);
        assert_eq!(decoded.max_retries, original.max_retries);
        assert_eq!(decoded.active, original.active);
        assert_eq!(decoded.created_at, original.created_at);
        assert_eq!(decoded.updated_at, original.updated_at);
    }

    #[test]
    fn config_serializes_all_expected_fields() {
        use serde_dynamo::aws_sdk_dynamodb_1::to_item;

        let config = sample_config();
        let item = to_item(&config).expect("serialization should succeed");

        assert!(item.contains_key("customer_id"), "missing customer_id");
        assert!(item.contains_key("url"), "missing url");
        assert!(item.contains_key("secret"), "missing secret");
        assert!(item.contains_key("max_retries"), "missing max_retries");
        assert!(item.contains_key("active"), "missing active");
        assert!(item.contains_key("created_at"), "missing created_at");
        assert!(item.contains_key("updated_at"), "missing updated_at");
    }

    #[test]
    fn config_to_response_copies_all_fields() {
        let config = sample_config();
        let resp = config.to_response();

        assert_eq!(resp.customer_id, config.customer_id);
        assert_eq!(resp.url, config.url);
        assert_eq!(resp.secret, config.secret);
        assert_eq!(resp.max_retries, config.max_retries);
        assert_eq!(resp.active, config.active);
        assert_eq!(resp.created_at, config.created_at);
        assert_eq!(resp.updated_at, config.updated_at);
    }
}
