//! Display hints from coin types and recorded metadata, never an endorsement registry.

pub fn display_name(asset: &str) -> String {
    let parts: Vec<_> = asset.split("::").collect();
    if parts.len() != 3 || parts[0].len() != 64 || !parts[0].bytes().all(|c| c.is_ascii_hexdigit())
    {
        return asset.to_owned();
    }
    // Keep package identity in the label: token symbols are not unique.
    // Generic COIN names were resolved with coinMetadata on 2026-10-19.
    // Provenance: docs/evidence/coin-display-names-2026-10-19.json.
    let name = match asset {
        "5d4b302506645c37ff133b98c4b50a5ae14841659738d6d733d59d0d217a93bf::coin::COIN" => "USDC",
        "af8cd5edc19c4512f4259f0bee101a40d41ebed738ade5874359610ef8eeced5::coin::COIN" => "WETH",
        "b7844e289a8410e50fb3ca48d69eb9cf29e27d223ef90353fe1bd8e27ff8f3f8::coin::COIN" => "SOL",
        "c060006111016b8a020ad5b33834984a437aaa7d3c74c18e09a95d48aceab08c::coin::COIN" => "USDT",
        _ => parts[2],
    };
    format!("{name} · {}…{}", &parts[0][..6], &parts[0][60..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_preserve_package_identity_and_handle_unknown_formats() {
        let first = format!("{}::usdc::USDC", "a".repeat(64));
        let second = format!("{}::usdc::USDC", "b".repeat(64));
        assert_ne!(display_name(&first), display_name(&second));
        assert_eq!(display_name("unknown"), "unknown");
        assert!(display_name(&first).starts_with("USDC · "));
        let weth = "af8cd5edc19c4512f4259f0bee101a40d41ebed738ade5874359610ef8eeced5::coin::COIN";
        assert!(display_name(weth).starts_with("WETH · "));
        assert!(!display_name(&weth.replace("coin::COIN", "other::COIN")).starts_with("WETH"));
    }
}
