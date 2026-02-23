use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Solana JSON-RPC client (Alchemy or any Solana RPC endpoint)
pub struct SolanaRpcClient {
    client: Client,
    url: String,
    chain_name: &'static str,
    max_retries: u32,
}

impl SolanaRpcClient {
    pub fn new(url: &str, chain_name: &'static str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(4)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            url: url.to_string(),
            chain_name,
            max_retries: 3,
        }
    }

    /// Make a single JSON-RPC call with retry logic
    async fn call(&self, method: &str, params: Value) -> Result<Value, String> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        });

        for attempt in 0..=self.max_retries {
            let resp = self
                .client
                .post(&self.url)
                .header("Content-Type", "application/json")
                .json(&body)
                .send()
                .await;

            match resp {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        if attempt < self.max_retries {
                            warn!(
                                "[{}] Solana RPC {} HTTP {} (attempt {}): {}",
                                self.chain_name,
                                method,
                                status,
                                attempt + 1,
                                &text[..text.len().min(200)]
                            );
                            sleep(Duration::from_millis(500 * (1 << attempt))).await;
                            continue;
                        }
                        return Err(format!("HTTP {}: {}", status, text));
                    }

                    let rpc_resp: RpcResponse = response
                        .json()
                        .await
                        .map_err(|e| format!("JSON parse error: {}", e))?;

                    if let Some(err) = rpc_resp.error {
                        return Err(format!("RPC error {}: {}", err.code, err.message));
                    }

                    return Ok(rpc_resp.result.unwrap_or(Value::Null));
                }
                Err(e) => {
                    if attempt < self.max_retries {
                        warn!(
                            "[{}] Solana RPC {} error (attempt {}): {}",
                            self.chain_name,
                            method,
                            attempt + 1,
                            e
                        );
                        sleep(Duration::from_millis(500 * (1 << attempt))).await;
                        continue;
                    }
                    return Err(format!("Request error: {}", e));
                }
            }
        }

        Err("Max retries exceeded".into())
    }

    /// getSlot → current slot number
    pub async fn get_slot(&self) -> Result<u64, String> {
        let result = self.call("getSlot", json!([])).await?;
        result
            .as_u64()
            .ok_or_else(|| "Invalid slot response".to_string())
    }

    /// getBlock(slot) → full block with transactions.
    /// Returns None for skipped/empty slots (normal on Solana).
    pub async fn get_block(&self, slot: u64) -> Result<Option<SolanaBlock>, String> {
        let params = json!([
            slot,
            {
                "encoding": "json",
                "maxSupportedTransactionVersion": 0,
                "transactionDetails": "full",
                "rewards": false
            }
        ]);

        match self.call("getBlock", params).await {
            Ok(value) => {
                if value.is_null() {
                    return Ok(None);
                }
                let block: SolanaBlock = serde_json::from_value(value)
                    .map_err(|e| format!("Block parse error: {}", e))?;
                Ok(Some(block))
            }
            Err(e) => {
                // Solana returns specific error codes for skipped/unavailable slots
                if e.contains("-32007")
                    || e.contains("-32009")
                    || e.contains("-32004")
                    || e.contains("was skipped")
                    || e.contains("not available")
                    || e.contains("Slot was skipped")
                {
                    debug!(
                        "[{}] Slot {} skipped/unavailable",
                        self.chain_name, slot
                    );
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }
}

// =============================================================================
// RPC Response Types
// =============================================================================

#[derive(Debug, Deserialize)]
struct RpcResponse {
    result: Option<Value>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
}

// =============================================================================
// Solana Data Types
// =============================================================================

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaBlock {
    pub block_time: Option<u64>,
    pub block_height: Option<u64>,
    pub parent_slot: u64,
    #[serde(default)]
    pub transactions: Vec<SolanaTransactionEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaTransactionEntry {
    pub transaction: SolanaTransaction,
    pub meta: Option<SolanaTransactionMeta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaTransaction {
    pub signatures: Vec<String>,
    pub message: SolanaMessage,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaMessage {
    /// Legacy transactions use accountKeys
    #[serde(default)]
    pub account_keys: Vec<AccountKeyEntry>,
}

/// Account key can be either a plain string (legacy) or an object with pubkey (versioned/parsed)
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum AccountKeyEntry {
    Plain(String),
    Parsed { pubkey: String },
}

impl AccountKeyEntry {
    pub fn to_pubkey(&self) -> &str {
        match self {
            AccountKeyEntry::Plain(s) => s,
            AccountKeyEntry::Parsed { pubkey } => pubkey,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SolanaTransactionMeta {
    pub err: Option<Value>,
    #[serde(default)]
    pub pre_balances: Vec<u64>,
    #[serde(default)]
    pub post_balances: Vec<u64>,
    #[serde(default)]
    pub pre_token_balances: Vec<TokenBalance>,
    #[serde(default)]
    pub post_token_balances: Vec<TokenBalance>,
    pub loaded_addresses: Option<LoadedAddresses>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenBalance {
    pub account_index: usize,
    pub mint: String,
    pub owner: Option<String>,
    pub ui_token_amount: UiTokenAmount,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UiTokenAmount {
    pub amount: String,
    pub decimals: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadedAddresses {
    #[serde(default)]
    pub writable: Vec<String>,
    #[serde(default)]
    pub readonly: Vec<String>,
}
