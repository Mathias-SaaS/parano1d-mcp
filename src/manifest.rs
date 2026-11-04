// SPDX-License-Identifier: Apache-2.0
//! The complete Parano1d JSON-RPC surface, one entry per method.
//!
//! Source of truth: `noid_rpc/src/api.rs` at tag v2.1.0 in `~/Documents/parano1d`.
//! v2.0.4 has 68 methods; v2.1.0 adds 11 (10 native + walletQueryReceipts).

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Access {
    /// Safe read. Available in every mode.
    Read,
    /// Changes the node wallet, submits funds/contracts/native operations, or writes local wallet state.
    Wallet,
    /// Stops the node or drives external mining. Only in `admin`.
    Admin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    /// No runtime capability gate.
    None,
    /// Requires `getContractProtocol.runtime_available`.
    ContractRuntime,
    /// Requires the node to expose native RPC (v2.1.x).
    NativeSupported,
    /// Requires the native protocol to be active (mainnet: H279,370).
    NativeActive,
}

#[derive(Debug, Clone, Copy)]
pub struct MethodSpec {
    /// RPC suffix without the `paranoid_` prefix.
    pub name: &'static str,
    pub access: Access,
    pub gate: Gate,
    pub min_params: usize,
    pub max_params: usize,
    /// Added in v2.1.0 (native layer or walletQueryReceipts).
    pub since_2_1: bool,
    pub summary: &'static str,
}

macro_rules! method {
    ($name:literal, $access:ident, $gate:ident, $min:literal, $max:literal, $since:literal, $summary:literal) => {
        MethodSpec {
            name: $name,
            access: Access::$access,
            gate: Gate::$gate,
            min_params: $min,
            max_params: $max,
            since_2_1: $since,
            summary: $summary,
        }
    };
}

/// All 79 methods of v2.1.0. Order mirrors the API trait.
pub static METHODS: &[MethodSpec] = &[
    // Native v2.1 ---------------------------------------------------------
    method!(
        "prepareNativeIntent",
        Read,
        NativeActive,
        2,
        3,
        true,
        "Prepare an unsigned native intent for client authorization"
    ),
    method!(
        "walletPreviewNative",
        Read,
        NativeActive,
        1,
        1,
        true,
        "Quote a native token/NFT/trade operation from the node wallet"
    ),
    method!(
        "walletSubmitNative",
        Wallet,
        NativeActive,
        2,
        2,
        true,
        "Submit a previewed native operation (spends NOID/tokens)"
    ),
    method!(
        "walletNativePending",
        Read,
        NativeSupported,
        0,
        0,
        true,
        "List pending or unconfirmed native wallet activity"
    ),
    method!(
        "walletDismissNativePending",
        Wallet,
        NativeSupported,
        1,
        1,
        true,
        "Dismiss a local native pending record"
    ),
    method!(
        "getNativeProtocol",
        Read,
        None,
        0,
        0,
        true,
        "Native v2.1 activation, limits and capacities"
    ),
    method!(
        "getNativeCatalog",
        Read,
        NativeSupported,
        3,
        3,
        true,
        "Browse markets, collections or NFT listings"
    ),
    method!(
        "getNativeMarket",
        Read,
        NativeSupported,
        1,
        1,
        true,
        "Current bonding curve / DEX pool for an asset"
    ),
    method!(
        "getNativeHoldings",
        Read,
        NativeSupported,
        3,
        3,
        true,
        "Typed live holdings for any owner"
    ),
    method!(
        "walletGetNativeHoldings",
        Read,
        NativeSupported,
        2,
        2,
        true,
        "Typed live holdings of the active wallet"
    ),
    // Contracts -----------------------------------------------------------
    method!(
        "getContractProtocol",
        Read,
        None,
        0,
        0,
        false,
        "Contract ABI, activation and class limits"
    ),
    method!(
        "previewObjectCall",
        Read,
        ContractRuntime,
        1,
        1,
        false,
        "Preview a contract call"
    ),
    method!(
        "createObject",
        Wallet,
        ContractRuntime,
        1,
        1,
        false,
        "Create public contract terms"
    ),
    method!(
        "getObjectStatus",
        Read,
        ContractRuntime,
        2,
        2,
        false,
        "Current status of one contract instance"
    ),
    method!(
        "getObjectInstances",
        Read,
        ContractRuntime,
        3,
        3,
        false,
        "List instances of an opening"
    ),
    method!(
        "walletFundObject",
        Wallet,
        ContractRuntime,
        3,
        4,
        false,
        "Fund a contract instance (spends NOID)"
    ),
    method!(
        "walletCallObject",
        Wallet,
        ContractRuntime,
        1,
        1,
        false,
        "Call a contract instance (may spend NOID)"
    ),
    method!(
        "walletGetObjectOpening",
        Read,
        ContractRuntime,
        1,
        1,
        false,
        "Resolve an object address to its opening"
    ),
    method!(
        "walletWatchObject",
        Wallet,
        ContractRuntime,
        1,
        1,
        false,
        "Retain an opening and capture future receipts"
    ),
    method!(
        "walletListObjectStates",
        Read,
        ContractRuntime,
        3,
        3,
        false,
        "Browse locally retained counter states"
    ),
    method!(
        "walletListObjectReceipts",
        Read,
        ContractRuntime,
        3,
        3,
        false,
        "Browse locally retained contract receipts"
    ),
    method!(
        "exportObjectReceipt",
        Read,
        ContractRuntime,
        2,
        2,
        false,
        "Export a contract receipt"
    ),
    method!(
        "verifyObjectReceipt",
        Read,
        ContractRuntime,
        1,
        1,
        false,
        "Verify a contract receipt"
    ),
    method!(
        "walletImportObjectReceipt",
        Wallet,
        ContractRuntime,
        1,
        2,
        false,
        "Verify and retain a received contract receipt"
    ),
    // Chain ---------------------------------------------------------------
    method!("blockCount", Read, None, 0, 0, false, "Tip height"),
    method!("getChainInfo", Read, None, 0, 0, false, "Chain tip summary"),
    method!(
        "getBlockHash",
        Read,
        None,
        1,
        1,
        false,
        "Permanent block hash at a height"
    ),
    method!(
        "getBlockHeader",
        Read,
        None,
        1,
        1,
        false,
        "Decoded permanent header at a height"
    ),
    method!(
        "getBlockHeaderByHash",
        Read,
        None,
        1,
        1,
        false,
        "Decoded permanent header by hash"
    ),
    method!(
        "getHeaderByHeight",
        Read,
        None,
        1,
        1,
        false,
        "Raw 212-byte header at a height"
    ),
    method!(
        "getHeaderByHash",
        Read,
        None,
        1,
        1,
        false,
        "Raw 212-byte header by hash"
    ),
    method!(
        "getHistoryStepTerminal",
        Read,
        None,
        0,
        0,
        false,
        "Current finalized history-step terminal"
    ),
    method!("getSlot", Read, None, 1, 1, false, "UTXO slot contents"),
    method!(
        "getSlotsByOwner",
        Read,
        None,
        1,
        1,
        false,
        "Live slots owned by an address"
    ),
    method!(
        "getActiveSlotCount",
        Read,
        None,
        0,
        0,
        false,
        "Total live UTXO count"
    ),
    method!(
        "getStateInfo",
        Read,
        None,
        0,
        0,
        false,
        "State capacity, occupancy and size"
    ),
    method!(
        "getStateMap",
        Read,
        None,
        0,
        0,
        false,
        "Bounded state occupancy atlas"
    ),
    method!(
        "getTx",
        Read,
        None,
        1,
        1,
        false,
        "Permanent transaction confirmation pointer"
    ),
    method!(
        "getBlock",
        Read,
        None,
        1,
        1,
        false,
        "Retained block body hex (42-block window)"
    ),
    method!(
        "getBlockDetails",
        Read,
        None,
        1,
        1,
        false,
        "Header plus retained body details"
    ),
    method!(
        "getRecentTransactions",
        Read,
        None,
        3,
        3,
        false,
        "Paginated retained transactions"
    ),
    // Network and mining --------------------------------------------------
    method!(
        "getMiningInfo",
        Read,
        None,
        0,
        0,
        false,
        "Difficulty, reward and active slots"
    ),
    method!(
        "getPeerCount",
        Read,
        None,
        0,
        0,
        false,
        "Connected peer count"
    ),
    method!(
        "getNodeStatus",
        Read,
        None,
        0,
        0,
        false,
        "Sync, mining and P2P status"
    ),
    method!(
        "estimateFee",
        Read,
        None,
        1,
        1,
        false,
        "Fee for one input and N outputs"
    ),
    method!(
        "estimateFeeDetailed",
        Read,
        None,
        2,
        2,
        false,
        "Detailed fee for N inputs and M outputs"
    ),
    // Utilities -----------------------------------------------------------
    method!(
        "validateAddress",
        Read,
        None,
        1,
        1,
        false,
        "Validate and normalise an address"
    ),
    method!(
        "getSlotHints",
        Read,
        None,
        1,
        1,
        false,
        "Candidate empty slot indices"
    ),
    method!(
        "getSlotHintsSalted",
        Read,
        None,
        2,
        2,
        false,
        "Salted candidate empty slot indices"
    ),
    method!(
        "getEpochAnchor",
        Read,
        None,
        0,
        0,
        false,
        "Current transaction epoch anchor"
    ),
    method!(
        "submitTxIntent",
        Wallet,
        None,
        1,
        1,
        false,
        "Submit a raw signed transaction intent"
    ),
    // Mempool -------------------------------------------------------------
    method!(
        "getMempoolInfo",
        Read,
        None,
        0,
        0,
        false,
        "Mempool count, floor and transactions"
    ),
    method!(
        "getMempoolSize",
        Read,
        None,
        0,
        0,
        false,
        "Pending transaction count"
    ),
    method!(
        "getMempoolStats",
        Read,
        None,
        0,
        0,
        false,
        "Mempool capacity and byte pressure"
    ),
    method!(
        "getMempoolEntry",
        Read,
        None,
        1,
        1,
        false,
        "One pending transaction"
    ),
    // Receipt -------------------------------------------------------------
    method!(
        "verifyReceipt",
        Read,
        None,
        1,
        1,
        false,
        "Verify a payment receipt"
    ),
    // External mining -----------------------------------------------------
    method!(
        "getBlockTemplate",
        Admin,
        None,
        1,
        1,
        false,
        "Request a single-use external mining template"
    ),
    method!(
        "submitBlock",
        Admin,
        None,
        2,
        2,
        false,
        "Submit a nonce for a prepared template"
    ),
    // Node control --------------------------------------------------------
    method!("stop", Admin, None, 0, 0, false, "Gracefully stop the node"),
    // Wallet --------------------------------------------------------------
    method!(
        "walletStatus",
        Read,
        None,
        0,
        0,
        false,
        "Wallet address, balance and UTXO count"
    ),
    method!(
        "walletGetAddress",
        Read,
        None,
        1,
        1,
        false,
        "Address at a key index"
    ),
    method!(
        "walletGetBalance",
        Read,
        None,
        0,
        0,
        false,
        "Balance breakdown"
    ),
    method!(
        "walletListUtxos",
        Read,
        None,
        0,
        0,
        false,
        "List confirmed wallet UTXOs"
    ),
    method!(
        "walletUtxoSnapshot",
        Read,
        None,
        0,
        1,
        false,
        "Conditional UTXO snapshot"
    ),
    method!(
        "walletHistory",
        Read,
        None,
        0,
        0,
        false,
        "Wallet transaction history"
    ),
    method!(
        "walletReceipts",
        Read,
        None,
        2,
        3,
        false,
        "Paginated saved payment receipts"
    ),
    method!(
        "walletQueryReceipts",
        Read,
        NativeSupported,
        3,
        3,
        true,
        "Filter the saved receipt library (v2.1)"
    ),
    method!(
        "walletMinedBlocks",
        Read,
        None,
        2,
        2,
        false,
        "Blocks mined by this wallet"
    ),
    method!(
        "walletScan",
        Wallet,
        None,
        0,
        0,
        false,
        "Reload the active owner from the durable index"
    ),
    method!(
        "walletDiscoverAddresses",
        Wallet,
        None,
        1,
        1,
        false,
        "Discover funded addresses after import"
    ),
    method!(
        "walletPlanSend",
        Read,
        None,
        3,
        3,
        false,
        "Dry-run a NOID send (no submission)"
    ),
    method!(
        "walletSend",
        Wallet,
        None,
        3,
        3,
        false,
        "Send NOID (spends funds)"
    ),
    method!(
        "walletPlanConsolidation",
        Read,
        None,
        0,
        0,
        false,
        "Quote a UTXO consolidation"
    ),
    method!(
        "walletConsolidate",
        Wallet,
        None,
        3,
        3,
        false,
        "Execute a quoted consolidation (spends fee)"
    ),
    method!(
        "walletExportReceipt",
        Read,
        None,
        1,
        1,
        false,
        "Export a saved payment receipt"
    ),
    method!(
        "walletNextAddress",
        Wallet,
        None,
        0,
        0,
        false,
        "Generate the next inactive address"
    ),
    method!(
        "walletListAddresses",
        Read,
        None,
        0,
        0,
        false,
        "List local addresses"
    ),
    method!(
        "walletActiveAddress",
        Read,
        None,
        0,
        0,
        false,
        "The active address"
    ),
    method!(
        "walletSetActiveAddress",
        Wallet,
        None,
        1,
        1,
        false,
        "Switch the active address"
    ),
];

pub fn find(name: &str) -> Option<&'static MethodSpec> {
    METHODS.iter().find(|method| method.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn has_exactly_79_unique_methods() {
        assert_eq!(METHODS.len(), 79);
        let unique: HashSet<_> = METHODS.iter().map(|m| m.name).collect();
        assert_eq!(unique.len(), 79);
    }

    #[test]
    fn v21_adds_exactly_eleven_methods() {
        let added: Vec<_> = METHODS
            .iter()
            .filter(|m| m.since_2_1)
            .map(|m| m.name)
            .collect();
        assert_eq!(added.len(), 11);
        for expected in [
            "prepareNativeIntent",
            "walletPreviewNative",
            "walletSubmitNative",
            "walletNativePending",
            "walletDismissNativePending",
            "getNativeProtocol",
            "getNativeCatalog",
            "getNativeMarket",
            "getNativeHoldings",
            "walletGetNativeHoldings",
            "walletQueryReceipts",
        ] {
            assert!(added.contains(&expected), "missing v2.1 method {expected}");
        }
    }

    #[test]
    fn access_classes_are_complete() {
        let read = METHODS.iter().filter(|m| m.access == Access::Read).count();
        let wallet = METHODS
            .iter()
            .filter(|m| m.access == Access::Wallet)
            .count();
        let admin = METHODS.iter().filter(|m| m.access == Access::Admin).count();
        assert_eq!((read, wallet, admin), (62, 14, 3));
    }

    #[test]
    fn param_bounds_are_sane() {
        for method in METHODS {
            assert!(method.min_params <= method.max_params, "{}", method.name);
            assert!(method.max_params <= 4, "{}", method.name);
        }
    }

    #[test]
    fn dangerous_methods_are_not_read() {
        for name in [
            "walletSend",
            "walletConsolidate",
            "submitTxIntent",
            "stop",
            "walletSubmitNative",
        ] {
            assert_ne!(find(name).unwrap().access, Access::Read, "{name}");
        }
    }
}
