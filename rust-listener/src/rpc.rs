use crate::types::{Block, Log, RpcResponse, TRANSFER_TOPIC};
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use thiserror::Error;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Internal type for parsing individual items in a JSON-RPC batch response
#[derive(Debug, Deserialize)]
struct BatchResponseItem {
    id: u64,
    result: Option<Value>,
    error: Option<BatchRpcError>,
}

/// RPC error within a batch response item
#[derive(Debug, Deserialize)]
struct BatchRpcError {
    code: i64,
    message: String,
}

#[derive(Error, Debug)]
pub enum RpcError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("RPC error: {0}")]
    Rpc(String),
    #[error("Parse error: {0}")]
    Parse(String),
    #[error("Rate limited after max retries")]
    RateLimited,
}

/// Generic JSON-RPC client for any Ethereum-compatible blockchain
/// Works with any provider: Alchemy, Infura, QuickNode, public RPCs, etc.
pub struct RpcClient {
    client: Client,
    url: String,
    chain_name: String,
    max_retries: u32,
    retry_base_delay_ms: u64,
}

impl RpcClient {
    /// Create a new RPC client
    ///
    /// # Arguments
    /// * `url` - Any Ethereum JSON-RPC endpoint URL (Alchemy, Infura, QuickNode, public RPC, etc.)
    /// * `chain_name` - Human-readable chain name for logging
    pub fn new(url: &str, chain_name: &str) -> Self {
        Self::with_config(url, chain_name, 3, 100, 30)
    }

    /// Create a new RPC client with custom retry configuration
    ///
    /// # Arguments
    /// * `url` - Any Ethereum JSON-RPC endpoint URL
    /// * `chain_name` - Human-readable chain name for logging
    /// * `max_retries` - Maximum number of retries on rate limit or transient errors
    /// * `retry_base_delay_ms` - Base delay in milliseconds for exponential backoff
    /// * `timeout_secs` - HTTP request timeout in seconds
    pub fn with_config(
        url: &str,
        chain_name: &str,
        max_retries: u32,
        retry_base_delay_ms: u64,
        timeout_secs: u64,
    ) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(timeout_secs))
            .pool_max_idle_per_host(2)         // Reduced from 5 to save memory
            .pool_idle_timeout(Duration::from_secs(30)) // Release idle connections after 30s
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            url: url.to_string(),
            chain_name: chain_name.to_string(),
            max_retries,
            retry_base_delay_ms,
        }
    }

    /// Check if an HTTP status code indicates a retryable error
    fn is_retryable_status(status: u16) -> bool {
        // 429 = Rate Limited
        // 502 = Bad Gateway
        // 503 = Service Unavailable
        // 504 = Gateway Timeout
        matches!(status, 429 | 502 | 503 | 504)
    }

    /// Make a JSON-RPC request with automatic retry on rate limit and transient errors
    async fn request<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        params: Value,
    ) -> Result<T, RpcError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params
        });

        let mut retries = 0;

        loop {
            let response = self
                .client
                .post(&self.url)
                .json(&body)
                .send()
                .await?;

            let status = response.status();

            // Handle retryable errors with exponential backoff
            if Self::is_retryable_status(status.as_u16()) {
                retries += 1;
                if retries > self.max_retries {
                    return Err(RpcError::RateLimited);
                }
                let delay = Duration::from_millis(
                    self.retry_base_delay_ms * 2u64.pow(retries - 1)
                );
                warn!(
                    "[{}] HTTP {} on {}, retry {}/{} in {:?}",
                    self.chain_name,
                    status.as_u16(),
                    method,
                    retries,
                    self.max_retries,
                    delay
                );
                sleep(delay).await;
                continue;
            }

            if !status.is_success() {
                return Err(RpcError::Rpc(format!(
                    "HTTP error {} from {}",
                    status, method
                )));
            }

            let rpc_response: RpcResponse<T> = response.json().await?;

            if let Some(error) = rpc_response.error {
                // Some providers return rate limit as RPC error rather than HTTP 429
                if error.code == -32005 || error.message.to_lowercase().contains("rate") {
                    retries += 1;
                    if retries > self.max_retries {
                        return Err(RpcError::RateLimited);
                    }
                    let delay = Duration::from_millis(
                        self.retry_base_delay_ms * 2u64.pow(retries - 1)
                    );
                    warn!(
                        "[{}] RPC rate limit on {}, retry {}/{} in {:?}",
                        self.chain_name,
                        method,
                        retries,
                        self.max_retries,
                        delay
                    );
                    sleep(delay).await;
                    continue;
                }

                return Err(RpcError::Rpc(format!(
                    "RPC error {}: {}",
                    error.code, error.message
                )));
            }

            return rpc_response
                .result
                .ok_or_else(|| RpcError::Parse("Missing result in RPC response".to_string()));
        }
    }

    /// Get the current block number (eth_blockNumber)
    pub async fn get_block_number(&self) -> Result<u64, RpcError> {
        let result: String = self.request("eth_blockNumber", json!([])).await?;
        u64::from_str_radix(result.trim_start_matches("0x"), 16)
            .map_err(|e| RpcError::Parse(format!("Invalid block number: {}", e)))
    }

    /// Get logs for Transfer events in a block range (eth_getLogs)
    ///
    /// Filters for ERC20 Transfer events only (topic[0] = Transfer signature)
    pub async fn get_transfer_logs(
        &self,
        from_block: u64,
        to_block: u64,
    ) -> Result<Vec<Log>, RpcError> {
        debug!(
            "[{}] Getting transfer logs from block {} to {}",
            self.chain_name, from_block, to_block
        );

        let params = json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "topics": [TRANSFER_TOPIC]
        }]);

        self.request("eth_getLogs", params).await
    }

    /// Get logs with custom filter (eth_getLogs)
    ///
    /// For advanced use cases where you need custom topic filtering
    pub async fn get_logs(
        &self,
        from_block: u64,
        to_block: u64,
        topics: Vec<Option<String>>,
    ) -> Result<Vec<Log>, RpcError> {
        let params = json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "topics": topics
        }]);

        self.request("eth_getLogs", params).await
    }

    /// Get logs from a specific contract address with topic filter (eth_getLogs)
    ///
    /// Used for fetching events from specific contracts like EscrowFactory
    pub async fn get_logs_by_address(
        &self,
        from_block: u64,
        to_block: u64,
        address: &str,
        topics: Vec<Option<String>>,
    ) -> Result<Vec<Log>, RpcError> {
        debug!(
            "[{}] Getting logs from {} for blocks {} to {}",
            self.chain_name, address, from_block, to_block
        );

        let params = json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "address": address,
            "topics": topics
        }]);

        self.request("eth_getLogs", params).await
    }

    /// Get logs with multiple possible topics (OR filter for topic[0])
    ///
    /// Used for fetching multiple event types in one call
    pub async fn get_logs_multi_topics(
        &self,
        from_block: u64,
        to_block: u64,
        address: &str,
        topic0_options: Vec<String>,
    ) -> Result<Vec<Log>, RpcError> {
        debug!(
            "[{}] Getting logs from {} with {} topic options for blocks {} to {}",
            self.chain_name, address, topic0_options.len(), from_block, to_block
        );

        let params = json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "address": address,
            "topics": [topic0_options]
        }]);

        self.request("eth_getLogs", params).await
    }

    /// Get logs with multiple possible topics without address filter (OR filter for topic[0])
    ///
    /// Used for fetching events from any contract matching the topics (e.g., escrow withdrawals)
    pub async fn get_logs_multi_topics_any_address(
        &self,
        from_block: u64,
        to_block: u64,
        topic0_options: Vec<String>,
    ) -> Result<Vec<Log>, RpcError> {
        debug!(
            "[{}] Getting logs with {} topic options (any address) for blocks {} to {}",
            self.chain_name, topic0_options.len(), from_block, to_block
        );

        let params = json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "topics": [topic0_options]
        }]);

        self.request("eth_getLogs", params).await
    }

    /// Get logs by a single topic without address filter
    ///
    /// Used for fetching events from any address matching a specific topic (e.g., EIP-7702 delegate events)
    pub async fn get_logs_by_topic_any_address(
        &self,
        from_block: u64,
        to_block: u64,
        topic0: &str,
    ) -> Result<Vec<Log>, RpcError> {
        debug!(
            "[{}] Getting logs for topic {} (any address) for blocks {} to {}",
            self.chain_name, topic0, from_block, to_block
        );

        let params = json!([{
            "fromBlock": format!("0x{:x}", from_block),
            "toBlock": format!("0x{:x}", to_block),
            "topics": [topic0]
        }]);

        self.request("eth_getLogs", params).await
    }

    /// Get block by number (eth_getBlockByNumber)
    ///
    /// Returns block header without transactions (for getting timestamp)
    pub async fn get_block(&self, block_number: u64) -> Result<Block, RpcError> {
        let params = json!([format!("0x{:x}", block_number), false]);
        self.request("eth_getBlockByNumber", params).await
    }

    /// Get the RPC endpoint URL (for logging/debugging)
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Get the chain name (for logging/debugging)
    pub fn chain_name(&self) -> &str {
        &self.chain_name
    }

    // =========================================================================
    // Batch JSON-RPC Methods
    // =========================================================================

    /// Send a batch of JSON-RPC calls in a single HTTP request.
    /// Returns results in the same order as input, matched by response ID.
    /// Retries the entire batch on HTTP 429/502/503/504 or batch-level rate limits.
    pub async fn batch_request_raw(
        &self,
        calls: &[(&str, Value)],
    ) -> Result<Vec<Result<Value, RpcError>>, RpcError> {
        if calls.is_empty() {
            return Ok(vec![]);
        }

        let batch_body: Value = calls
            .iter()
            .enumerate()
            .map(|(i, (method, params))| {
                json!({
                    "jsonrpc": "2.0",
                    "id": (i + 1) as u64,
                    "method": method,
                    "params": params,
                })
            })
            .collect();

        let num_calls = calls.len();
        let mut retries = 0u32;

        loop {
            let response = self
                .client
                .post(&self.url)
                .json(&batch_body)
                .send()
                .await?;

            let status = response.status();

            // Retry on transient HTTP errors (same as single request)
            if Self::is_retryable_status(status.as_u16()) {
                retries += 1;
                if retries > self.max_retries {
                    return Err(RpcError::RateLimited);
                }
                let delay = Duration::from_millis(
                    self.retry_base_delay_ms * 2u64.pow(retries - 1),
                );
                warn!(
                    "[{}] HTTP {} on batch ({} calls), retry {}/{} in {:?}",
                    self.chain_name, status.as_u16(), num_calls,
                    retries, self.max_retries, delay
                );
                sleep(delay).await;
                continue;
            }

            if !status.is_success() {
                return Err(RpcError::Rpc(format!(
                    "HTTP error {} from batch ({} calls)",
                    status, num_calls
                )));
            }

            // Parse raw text to handle both array and object responses
            let raw_text = response.text().await
                .map_err(|e| RpcError::Parse(format!("Failed to read batch response: {}", e)))?;

            let raw_value: Value = serde_json::from_str(&raw_text)
                .map_err(|e| RpcError::Parse(format!("Invalid JSON in batch response: {}", e)))?;

            // Edge case: provider returns a single error object instead of array
            if let Value::Object(ref map) = raw_value {
                if let Some(err_obj) = map.get("error") {
                    let code = err_obj.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
                    let message = err_obj
                        .get("message")
                        .and_then(|m| m.as_str())
                        .unwrap_or("unknown error")
                        .to_string();

                    if code == -32005 || message.to_lowercase().contains("rate") {
                        retries += 1;
                        if retries > self.max_retries {
                            return Err(RpcError::RateLimited);
                        }
                        let delay = Duration::from_millis(
                            self.retry_base_delay_ms * 2u64.pow(retries - 1),
                        );
                        warn!(
                            "[{}] RPC rate limit on batch, retry {}/{} in {:?}",
                            self.chain_name, retries, self.max_retries, delay
                        );
                        sleep(delay).await;
                        continue;
                    }

                    return Err(RpcError::Rpc(format!(
                        "Batch rejected: RPC error {}: {}",
                        code, message
                    )));
                }
                return Err(RpcError::Parse(
                    "Expected JSON array for batch response, got object".to_string(),
                ));
            }

            // Normal case: response is a JSON array
            let items: Vec<BatchResponseItem> = serde_json::from_value(raw_value)
                .map_err(|e| RpcError::Parse(format!("Failed to parse batch response: {}", e)))?;

            // Check if ALL items are rate-limited → retry entire batch
            let all_rate_limited = !items.is_empty()
                && items.iter().all(|item| {
                    item.error.as_ref().map_or(false, |e| {
                        e.code == -32005 || e.message.to_lowercase().contains("rate")
                    })
                });

            if all_rate_limited {
                retries += 1;
                if retries > self.max_retries {
                    return Err(RpcError::RateLimited);
                }
                let delay = Duration::from_millis(
                    self.retry_base_delay_ms * 2u64.pow(retries - 1),
                );
                warn!(
                    "[{}] All {} batch items rate-limited, retry {}/{} in {:?}",
                    self.chain_name, num_calls, retries, self.max_retries, delay
                );
                sleep(delay).await;
                continue;
            }

            // Build results in input order, matched by ID (1-indexed)
            let mut results: Vec<Result<Value, RpcError>> = (0..num_calls)
                .map(|_| Err(RpcError::Parse("Missing response for batch item".to_string())))
                .collect();

            for item in items {
                let idx = match (item.id as usize).checked_sub(1) {
                    Some(i) if i < num_calls => i,
                    _ => {
                        warn!(
                            "[{}] Batch response id {} out of range (max {})",
                            self.chain_name, item.id, num_calls
                        );
                        continue;
                    }
                };

                if let Some(error) = item.error {
                    results[idx] = Err(RpcError::Rpc(format!(
                        "RPC error {}: {}",
                        error.code, error.message
                    )));
                } else if let Some(result) = item.result {
                    results[idx] = Ok(result);
                }
                // If neither result nor error, the default "Missing response" error stays
            }

            return Ok(results);
        }
    }

    /// Fetch logs for multiple filter objects in a single batch request.
    /// Returns Vec<Result<Vec<Log>>> in the same order as input filters.
    pub async fn batch_get_logs(
        &self,
        filters: &[Value],
    ) -> Result<Vec<Result<Vec<Log>, RpcError>>, RpcError> {
        debug!(
            "[{}] Batch getLogs with {} filters",
            self.chain_name, filters.len()
        );

        let calls: Vec<(&str, Value)> = filters
            .iter()
            .map(|filter| ("eth_getLogs", json!([filter])))
            .collect();

        let raw_results = self.batch_request_raw(&calls).await?;

        Ok(raw_results
            .into_iter()
            .map(|r| match r {
                Ok(value) => serde_json::from_value::<Vec<Log>>(value)
                    .map_err(|e| RpcError::Parse(format!("Failed to parse logs: {}", e))),
                Err(e) => Err(e),
            })
            .collect())
    }

    /// Fetch block headers for multiple block numbers in a single batch request.
    /// Returns Vec<Result<Block>> in the same order as input block numbers.
    pub async fn batch_get_blocks(
        &self,
        block_numbers: &[u64],
    ) -> Result<Vec<Result<Block, RpcError>>, RpcError> {
        if block_numbers.is_empty() {
            return Ok(vec![]);
        }

        debug!(
            "[{}] Batch getBlockByNumber for {} blocks",
            self.chain_name, block_numbers.len()
        );

        let calls: Vec<(&str, Value)> = block_numbers
            .iter()
            .map(|&num| {
                (
                    "eth_getBlockByNumber",
                    json!([format!("0x{:x}", num), false]),
                )
            })
            .collect();

        let raw_results = self.batch_request_raw(&calls).await?;

        Ok(raw_results
            .into_iter()
            .map(|r| match r {
                Ok(value) => serde_json::from_value::<Block>(value)
                    .map_err(|e| RpcError::Parse(format!("Failed to parse block: {}", e))),
                Err(e) => Err(e),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_retryable_status() {
        assert!(RpcClient::is_retryable_status(429));
        assert!(RpcClient::is_retryable_status(502));
        assert!(RpcClient::is_retryable_status(503));
        assert!(RpcClient::is_retryable_status(504));
        assert!(!RpcClient::is_retryable_status(200));
        assert!(!RpcClient::is_retryable_status(400));
        assert!(!RpcClient::is_retryable_status(500));
    }
}
