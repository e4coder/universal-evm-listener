use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, warn};

/// Bitcoin Core JSON-RPC client with HTTP Basic Auth
pub struct BitcoinRpcClient {
    client: Client,
    url: String,
    auth_header: Option<String>, // None for Alchemy (key in URL), Some for Bitcoin Core (Basic Auth)
    chain_name: &'static str,
    max_retries: u32,
}

impl BitcoinRpcClient {
    pub fn new(url: &str, user: &str, password: &str, chain_name: &'static str) -> Self {
        let auth_header = if user.is_empty() && password.is_empty() {
            None // Alchemy-style: API key in URL, no auth header
        } else {
            let credentials = format!("{}:{}", user, password);
            let encoded = base64_encode(&credentials);
            Some(format!("Basic {}", encoded))
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(4)
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            url: url.to_string(),
            auth_header,
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
            let mut req = self
                .client
                .post(&self.url)
                .header("Content-Type", "application/json");
            if let Some(auth) = &self.auth_header {
                req = req.header("Authorization", auth);
            }
            let resp = req.json(&body).send().await;

            match resp {
                Ok(response) => {
                    if !response.status().is_success() {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        if attempt < self.max_retries {
                            warn!(
                                "[{}] Bitcoin RPC {} HTTP {} (attempt {}): {}",
                                self.chain_name,
                                method,
                                status,
                                attempt + 1,
                                text
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
                            "[{}] Bitcoin RPC {} error (attempt {}): {}",
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

    /// Make a batch JSON-RPC call
    async fn batch_call(&self, requests: Vec<(String, Value)>) -> Result<Vec<Value>, String> {
        if requests.is_empty() {
            return Ok(vec![]);
        }

        let body: Vec<Value> = requests
            .iter()
            .enumerate()
            .map(|(i, (method, params))| {
                json!({
                    "jsonrpc": "2.0",
                    "id": i,
                    "method": method,
                    "params": params,
                })
            })
            .collect();

        let mut req = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json");
        if let Some(auth) = &self.auth_header {
            req = req.header("Authorization", auth);
        }
        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Batch request error: {}", e))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(format!("Batch HTTP {}: {}", status, text));
        }

        let items: Vec<BatchResponseItem> = resp
            .json()
            .await
            .map_err(|e| format!("Batch JSON parse error: {}", e))?;

        // Sort by ID to maintain request order
        let mut sorted = items;
        sorted.sort_by_key(|item| item.id);

        let mut results = Vec::with_capacity(sorted.len());
        for item in sorted {
            if let Some(err) = item.error {
                // Return error result as null (caller handles missing items)
                debug!(
                    "[{}] Batch item {} error: {} {}",
                    self.chain_name, item.id, err.code, err.message
                );
                results.push(Value::Null);
            } else {
                results.push(item.result.unwrap_or(Value::Null));
            }
        }

        Ok(results)
    }

    /// getblockcount → current block height
    pub async fn get_block_count(&self) -> Result<u64, String> {
        let result = self.call("getblockcount", json!([])).await?;
        result
            .as_u64()
            .ok_or_else(|| "Invalid block count response".to_string())
    }

    /// getblockhash(height) → block hash
    pub async fn get_block_hash(&self, height: u64) -> Result<String, String> {
        let result = self.call("getblockhash", json!([height])).await?;
        result
            .as_str()
            .map(|s| s.to_string())
            .ok_or_else(|| "Invalid block hash response".to_string())
    }

    /// getblock(hash, verbosity=2) → block with decoded transactions
    pub async fn get_block_verbose(&self, hash: &str) -> Result<BitcoinBlock, String> {
        let result = self.call("getblock", json!([hash, 2])).await?;
        serde_json::from_value(result).map_err(|e| format!("Block parse error: {}", e))
    }

    /// getrawmempool → list of txids in mempool
    pub async fn get_raw_mempool(&self) -> Result<Vec<String>, String> {
        let result = self.call("getrawmempool", json!([])).await?;
        serde_json::from_value(result).map_err(|e| format!("Mempool parse error: {}", e))
    }

    /// getrawtransaction(txid, verbose=true) → decoded transaction
    pub async fn get_raw_transaction(&self, txid: &str) -> Result<Option<BitcoinTransaction>, String> {
        match self.call("getrawtransaction", json!([txid, true])).await {
            Ok(result) => {
                let tx = serde_json::from_value(result)
                    .map_err(|e| format!("TX parse error: {}", e))?;
                Ok(Some(tx))
            }
            Err(e) => {
                // Transaction not found is expected for dropped mempool txs
                if e.contains("-5") || e.contains("No such mempool") || e.contains("not found") {
                    Ok(None)
                } else {
                    Err(e)
                }
            }
        }
    }

    /// Batch-fetch multiple transactions in a single HTTP request
    pub async fn batch_get_transactions(
        &self,
        txids: &[String],
    ) -> Result<Vec<Option<BitcoinTransaction>>, String> {
        if txids.is_empty() {
            return Ok(vec![]);
        }

        // Process in chunks to avoid oversized requests
        let mut all_results = Vec::with_capacity(txids.len());

        for chunk in txids.chunks(50) {
            let requests: Vec<(String, Value)> = chunk
                .iter()
                .map(|txid| ("getrawtransaction".to_string(), json!([txid, true])))
                .collect();

            let results = self.batch_call(requests).await?;

            for result in results {
                if result.is_null() {
                    all_results.push(None);
                } else {
                    match serde_json::from_value::<BitcoinTransaction>(result) {
                        Ok(tx) => all_results.push(Some(tx)),
                        Err(_) => all_results.push(None),
                    }
                }
            }
        }

        Ok(all_results)
    }
}

// =============================================================================
// Bitcoin RPC Response Types
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

#[derive(Debug, Deserialize)]
struct BatchResponseItem {
    id: u64,
    result: Option<Value>,
    error: Option<BatchRpcError>,
}

#[derive(Debug, Deserialize)]
struct BatchRpcError {
    code: i64,
    message: String,
}

// =============================================================================
// Bitcoin Data Types
// =============================================================================

#[derive(Debug, Deserialize)]
pub struct BitcoinBlock {
    pub height: u64,
    pub hash: String,
    pub time: u64,
    pub tx: Vec<BitcoinTransaction>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BitcoinTransaction {
    pub txid: String,
    pub vin: Vec<TxInput>,
    pub vout: Vec<TxOutput>,
    pub confirmations: Option<u64>,
    pub blockhash: Option<String>,
    #[serde(rename = "blocktime")]
    pub block_time: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxInput {
    pub txid: Option<String>,
    pub vout: Option<u32>,
    pub prevout: Option<PrevOut>,
    /// Present in coinbase transactions
    pub coinbase: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PrevOut {
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: Option<ScriptPubKey>,
    pub value: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxOutput {
    pub value: f64,
    pub n: u32,
    #[serde(rename = "scriptPubKey")]
    pub script_pub_key: ScriptPubKey,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ScriptPubKey {
    pub address: Option<String>,
    #[serde(rename = "type")]
    pub script_type: Option<String>,
}

// =============================================================================
// Base64 encoding (minimal, avoids extra dependency)
// =============================================================================

fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::with_capacity((bytes.len() + 2) / 3 * 4);

    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }

    result
}
