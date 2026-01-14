use crate::types::{DstEscrowCreatedData, SrcEscrowCreatedData};
use sha3::{Digest, Keccak256};

/// Decode SrcEscrowCreated event data
///
/// Event data layout (13 words × 32 bytes):
/// Word 0: orderHash
/// Word 1: hashlock
/// Word 2: srcMaker (address in lower 160 bits)
/// Word 3: srcTaker (address in lower 160 bits)
/// Word 4: srcToken (address in lower 160 bits)
/// Word 5: srcAmount
/// Word 6: srcSafetyDeposit
/// Word 7: srcTimelocks
/// Word 8: dstMaker (address in lower 160 bits)
/// Word 9: dstAmount
/// Word 10: dstToken (address in lower 160 bits)
/// Word 11: dstSafetyDeposit
/// Word 12: dstChainId
pub fn decode_src_escrow_created(data: &str) -> Option<SrcEscrowCreatedData> {
    let hex = data.strip_prefix("0x").unwrap_or(data);

    // Need at least 13 words (13 * 64 hex chars)
    if hex.len() < 13 * 64 {
        return None;
    }

    let get_word = |idx: usize| -> &str {
        &hex[idx * 64..(idx + 1) * 64]
    };

    let to_address = |word: &str| -> String {
        format!("0x{}", &word[24..].to_lowercase()) // Last 40 chars, lowercased
    };

    let to_bytes32 = |word: &str| -> String {
        format!("0x{}", word.to_lowercase())
    };

    // Parse dst_chain_id from hex
    let dst_chain_id = u32::from_str_radix(get_word(12), 16).unwrap_or(0);

    Some(SrcEscrowCreatedData {
        order_hash: to_bytes32(get_word(0)),
        hashlock: to_bytes32(get_word(1)),
        src_maker: to_address(get_word(2)),
        src_taker: to_address(get_word(3)),
        src_token: to_address(get_word(4)),
        src_amount: to_bytes32(get_word(5)),
        src_safety_deposit: to_bytes32(get_word(6)),
        src_timelocks: to_bytes32(get_word(7)),
        dst_maker: to_address(get_word(8)),
        dst_amount: to_bytes32(get_word(9)),
        dst_token: to_address(get_word(10)),
        dst_safety_deposit: to_bytes32(get_word(11)),
        dst_chain_id,
    })
}

/// Decode DstEscrowCreated event data
///
/// Event data layout (8 words × 32 bytes):
/// Word 0: orderHash
/// Word 1: hashlock
/// Word 2: dstMaker (address in lower 160 bits)
/// Word 3: dstTaker (address in lower 160 bits)
/// Word 4: dstToken (address in lower 160 bits)
/// Word 5: dstAmount
/// Word 6: dstSafetyDeposit
/// Word 7: dstTimelocks
pub fn decode_dst_escrow_created(data: &str) -> Option<DstEscrowCreatedData> {
    let hex = data.strip_prefix("0x").unwrap_or(data);

    // Need at least 8 words
    if hex.len() < 8 * 64 {
        return None;
    }

    let get_word = |idx: usize| -> &str {
        &hex[idx * 64..(idx + 1) * 64]
    };

    let to_address = |word: &str| -> String {
        format!("0x{}", &word[24..].to_lowercase())
    };

    let to_bytes32 = |word: &str| -> String {
        format!("0x{}", word.to_lowercase())
    };

    Some(DstEscrowCreatedData {
        order_hash: to_bytes32(get_word(0)),
        hashlock: to_bytes32(get_word(1)),
        dst_maker: to_address(get_word(2)),
        dst_taker: to_address(get_word(3)),
        dst_token: to_address(get_word(4)),
        dst_amount: to_bytes32(get_word(5)),
        dst_safety_deposit: to_bytes32(get_word(6)),
        dst_timelocks: to_bytes32(get_word(7)),
    })
}

/// Decode EscrowWithdrawal event data to extract the secret
///
/// Event data layout (1 word × 32 bytes):
/// Word 0: secret
pub fn decode_escrow_withdrawal(data: &str) -> Option<String> {
    let hex = data.strip_prefix("0x").unwrap_or(data);

    if hex.len() < 64 {
        return None;
    }

    Some(format!("0x{}", &hex[0..64].to_lowercase()))
}

/// Compute hashlock from secret using keccak256
/// hashlock = keccak256(secret)
pub fn compute_hashlock_from_secret(secret: &str) -> Option<String> {
    let secret_hex = secret.strip_prefix("0x").unwrap_or(secret);

    // Decode the secret from hex to bytes
    let secret_bytes = hex::decode(secret_hex).ok()?;

    // Compute keccak256 hash
    let mut hasher = Keccak256::new();
    hasher.update(&secret_bytes);
    let result = hasher.finalize();

    Some(format!("0x{}", hex::encode(result)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_src_escrow_created() {
        // Example from Arbitrum tx: 0x8b7ef790b3c541d753996bb014261fff900377a933bc19d3a8aaaa30d6c359cb
        let data = "0x169c0db441eaf375fc6dd71f7f81d684ddbe8c751c68dd87dddf5032aaafafa9b80a9e9053b23333887b6047be5ac6d3f62175a993ed349bd2bf92bf95fa0ce700000000000000000000000087f0f4b7e0c4a8d9e93e4c7e2b1b4f3d3a8c5d6e000000000000000000000000resolver000000000000000000000000000000000000000000000000000000000af88d065e77c8cc2239327c5edb3a432268e583100000000000000000000000000000000000000000000000000000000001e848000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000067890abc00000000000000000000000087f0f4b7e0c4a8d9e93e4c7e2b1b4f3d3a8c5d6e00000000000000000000000000000000000000000000000000000000001dcd6500000000000000000000000833589fcd6edb6e08f4c7c32d4f71b54bda02913000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000021a5";

        let result = decode_src_escrow_created(data);
        assert!(result.is_some());

        let parsed = result.unwrap();
        assert_eq!(parsed.order_hash, "0x169c0db441eaf375fc6dd71f7f81d684ddbe8c751c68dd87dddf5032aaafafa9");
        assert_eq!(parsed.dst_chain_id, 8613); // 0x21a5
    }

    #[test]
    fn test_decode_escrow_withdrawal() {
        let data = "0xe9af1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab";

        let result = decode_escrow_withdrawal(data);
        assert!(result.is_some());
        assert_eq!(result.unwrap(), "0xe9af1234567890abcdef1234567890abcdef1234567890abcdef1234567890ab");
    }
}
