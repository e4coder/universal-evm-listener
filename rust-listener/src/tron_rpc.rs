use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::time::Duration;
use tokio::time::sleep;
use tracing::warn;

/// Tron HTTP API client (Alchemy or TronGrid-compatible endpoint).
/// Uses the native Tron REST API: /wallet/getnowblock, /wallet/getblockbynum, etc.
/// Auth is via the API key embedded in the URL path (Alchemy style).
pub struct TronRpcClient {
    client: Client,
    url: String,
    chain_name: &'static str,
    max_retries: u32,
}

impl TronRpcClient {
    pub fn new(url: &str, chain_name: &'static str) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(4)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            url: url.trim_end_matches('/').to_string(),
            chain_name,
            max_retries: 3,
        }
    }

    /// POST to a Tron API path with JSON body and retry logic
    async fn post(&self, path: &str, body: &Value) -> Result<Value, String> {
        let url = format!("{}/{}", self.url, path);

        for attempt in 0..=self.max_retries {
            let resp = self
                .client
                .post(&url)
                .header("Content-Type", "application/json")
                .json(body)
                .send()
                .await;

            match resp {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        if attempt < self.max_retries {
                            warn!(
                                "[{}] Tron API {} HTTP {} (attempt {}): {}",
                                self.chain_name,
                                path,
                                status,
                                attempt + 1,
                                &text[..text.len().min(200)]
                            );
                            sleep(Duration::from_millis(500 * (1 << attempt))).await;
                            continue;
                        }
                        return Err(format!("HTTP {}: {}", status, text));
                    }

                    let value: Value = response
                        .json()
                        .await
                        .map_err(|e| format!("JSON parse error: {}", e))?;

                    return Ok(value);
                }
                Err(e) => {
                    if attempt < self.max_retries {
                        warn!(
                            "[{}] Tron API {} error (attempt {}): {}",
                            self.chain_name,
                            path,
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

    /// Get the current block number
    pub async fn get_now_block(&self) -> Result<u64, String> {
        let resp = self.post("wallet/getnowblock", &json!({})).await?;
        resp.pointer("/block_header/raw_data/number")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| "Invalid getnowblock response: missing block number".to_string())
    }

    /// Get a block by number (contains transactions with contract data for native TRX)
    pub async fn get_block_by_num(&self, num: u64) -> Result<TronBlock, String> {
        let resp = self
            .post("wallet/getblockbynum", &json!({"num": num}))
            .await?;
        serde_json::from_value(resp).map_err(|e| format!("Block parse error at {}: {}", num, e))
    }

    /// Get transaction info for all txs in a block (contains event logs for TRC20)
    pub async fn get_transaction_info_by_block_num(
        &self,
        num: u64,
    ) -> Result<Vec<TronTransactionInfo>, String> {
        let resp = self
            .post(
                "wallet/gettransactioninfobyblocknum",
                &json!({"num": num}),
            )
            .await?;

        // Empty blocks return an empty array
        if resp.is_array() && resp.as_array().map(|a| a.is_empty()).unwrap_or(false) {
            return Ok(vec![]);
        }

        serde_json::from_value(resp)
            .map_err(|e| format!("TxInfo parse error at block {}: {}", num, e))
    }
}

// =============================================================================
// Tron Data Types
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct TronBlock {
    #[serde(rename = "blockID")]
    pub block_id: Option<String>,
    pub block_header: TronBlockHeader,
    #[serde(default)]
    pub transactions: Vec<TronTransaction>,
}

#[derive(Debug, Deserialize)]
pub struct TronBlockHeader {
    pub raw_data: TronBlockHeaderRawData,
}

#[derive(Debug, Deserialize)]
pub struct TronBlockHeaderRawData {
    pub number: Option<u64>,
    #[serde(default)]
    pub timestamp: u64, // milliseconds
}

#[derive(Debug, Deserialize)]
pub struct TronTransaction {
    #[serde(rename = "txID")]
    pub tx_id: String,
    pub raw_data: TronTransactionRawData,
    #[serde(default)]
    pub ret: Vec<TronTransactionResult>,
}

#[derive(Debug, Deserialize)]
pub struct TronTransactionRawData {
    #[serde(default)]
    pub contract: Vec<TronContract>,
}

#[derive(Debug, Deserialize)]
pub struct TronContract {
    #[serde(rename = "type")]
    pub contract_type: String,
    pub parameter: TronContractParameter,
}

#[derive(Debug, Deserialize)]
pub struct TronContractParameter {
    pub value: Value,
    #[serde(rename = "type_url")]
    pub type_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TronTransactionResult {
    #[serde(rename = "contractRet")]
    pub contract_ret: Option<String>,
}

// From /wallet/gettransactioninfobyblocknum
#[derive(Debug, Deserialize)]
pub struct TronTransactionInfo {
    pub id: Option<String>,
    #[serde(rename = "blockNumber")]
    pub block_number: Option<u64>,
    #[serde(rename = "blockTimeStamp")]
    pub block_timestamp: Option<u64>, // milliseconds
    #[serde(default)]
    pub log: Vec<TronEventLog>,
    pub receipt: Option<TronReceipt>,
}

#[derive(Debug, Deserialize)]
pub struct TronEventLog {
    pub address: Option<String>, // hex without 0x, 41-prefix
    #[serde(default)]
    pub topics: Vec<String>, // hex without 0x
    pub data: Option<String>, // hex without 0x
}

#[derive(Debug, Deserialize)]
pub struct TronReceipt {
    pub result: Option<String>, // "SUCCESS" or other
}

// =============================================================================
// Address Conversion: Hex → Tron Base58Check (T-prefix)
// =============================================================================

/// Convert a hex address (41-prefix 21 bytes or 40 hex chars) to Tron Base58Check T-address.
pub fn hex_to_tron_address(hex_str: &str) -> Result<String, String> {
    let hex = hex_str.trim_start_matches("0x");

    // Build the 21-byte address with 0x41 prefix
    let addr_bytes = if hex.len() == 42 && hex.starts_with("41") {
        // Already has 41 prefix
        hex::decode(hex).map_err(|e| format!("hex decode error: {}", e))?
    } else if hex.len() == 40 {
        // 20-byte EVM-style, prepend 41
        let mut bytes = vec![0x41];
        bytes.extend(hex::decode(hex).map_err(|e| format!("hex decode error: {}", e))?);
        bytes
    } else {
        return Err(format!("Invalid hex address length: {}", hex.len()));
    };

    if addr_bytes.len() != 21 {
        return Err(format!(
            "Expected 21 address bytes, got {}",
            addr_bytes.len()
        ));
    }

    // Double SHA-256 checksum (first 4 bytes)
    let hash1 = Sha256::digest(&addr_bytes);
    let hash2 = Sha256::digest(&hash1);
    let checksum = &hash2[..4];

    // Append checksum → 25 bytes → Base58 encode
    let mut full = addr_bytes;
    full.extend_from_slice(checksum);

    Ok(bs58::encode(full).into_string())
}
