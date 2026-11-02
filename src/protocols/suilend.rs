//! Suilend Move type routing. Incremental decoding lives in suilend_incremental.
use super::AdapterManifest;

/// Verified against @suilend/sdk 14.2.0; see docs/evidence/sdk-live-scope-2026-11-02.json.
pub const MAIN_MARKET_ID: &str =
    "0x84030d26d85eaa7035084a057f2f11f701b7e2e4eda87551becbc7c97505ece1";
pub const SUI: &str = "0x2::sui::SUI";
pub const NATIVE_USDC: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";

pub const MANIFEST: AdapterManifest = AdapterManifest {
    protocol_id: "suilend",
    adapter_version: env!("CARGO_PKG_VERSION"),
    // Cached, unverified lending-health diagnostics only - see
    // docs/suilend-notes.md for why fresh valuation remains blocked.
    capabilities: &["cached_lending_health"],
};

/// The GraphQL type filter `ingest::snapshot::discover` pages over to find
/// every `LendingMarket<...>` instance of a deployed package - deliberately
/// without a trailing `<` or `>`, since the discovery query matches on the
/// generic type's origin, not one fully-instantiated type argument.
pub fn market_type_filter(package: &str) -> String {
    format!("{package}::lending_market::LendingMarket")
}

/// The fully-instantiated market type prefix used to parse one already-
/// fetched market object's type argument back out (see
/// [`market_type_argument`]).
fn market_type_prefix(package: &str) -> String {
    format!("{package}::lending_market::LendingMarket<")
}

/// Extract `P` from a market object's Move type
/// `{package}::lending_market::LendingMarket<P>`. `None` means the object's
/// type does not match this adapter's expected shape at all - the caller
/// treats that as an unsupported/unexpected deployment, not a parse retry.
pub fn market_type_argument<'a>(move_type: &'a str, package: &str) -> Option<&'a str> {
    move_type
        .strip_prefix(&market_type_prefix(package))
        .and_then(|s| s.strip_suffix('>'))
}

pub fn obligation_type(package: &str, type_argument: &str) -> String {
    format!("{package}::obligation::Obligation<{type_argument}>")
}

pub fn cap_type(package: &str, type_argument: &str) -> String {
    format!("{package}::lending_market::ObligationOwnerCap<{type_argument}>")
}

/// Validate position references without cloning the market's large reserve book.
pub fn validate_position(
    object: &crate::model::Object,
    market: &crate::model::Object,
    package: &str,
    type_argument: &str,
) -> anyhow::Result<()> {
    use crate::model::{array, integer, string};
    use anyhow::{Context, ensure};
    object.validate()?;
    ensure!(
        object.move_type == crate::protocols::suilend::obligation_type(package, type_argument),
        "unexpected obligation type"
    );
    ensure!(
        string(&object.contents, "lending_market_id")? == market.address,
        "obligation belongs to another market"
    );
    let reserves = array(&market.contents, "reserves")?;
    for side in ["deposits", "borrows"] {
        for leg in array(&object.contents, side)? {
            let index = usize::try_from(integer(leg, "reserve_array_index")?)?;
            let reserve = reserves.get(index).context("unknown reserve index")?;
            ensure!(
                integer(reserve, "array_index")? == index as u64,
                "reserve order mismatch"
            );
            ensure!(
                string(leg, "coin_type")? == string(reserve, "coin_type")?,
                "reserve coin type mismatch"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn market_type_argument_round_trips_through_the_filter_and_prefix() {
        let package = "0xabc";
        let type_argument = "0xdef::pool::MAIN";
        let full_type = format!("{}<{type_argument}>", market_type_filter(package));
        assert_eq!(
            market_type_argument(&full_type, package),
            Some(type_argument)
        );
    }

    #[test]
    fn market_type_argument_rejects_an_unsupported_or_unrelated_package() {
        let type_argument = "0xdef::pool::MAIN";
        let full_type = format!("{}<{type_argument}>", market_type_filter("0xabc"));
        // A different package id (e.g. an upgraded deployment this
        // adapter was not told about) must not parse - silently accepting
        // it would be exactly the "accept a new package because its
        // fields look familiar" failure temp.md section 2 forbids.
        assert_eq!(market_type_argument(&full_type, "0xdifferent"), None);
    }

    #[test]
    fn obligation_and_cap_type_embed_the_market_type_argument() {
        let package = "0xabc";
        let type_argument = "0xdef::pool::MAIN";
        assert_eq!(
            obligation_type(package, type_argument),
            "0xabc::obligation::Obligation<0xdef::pool::MAIN>"
        );
        assert_eq!(
            cap_type(package, type_argument),
            "0xabc::lending_market::ObligationOwnerCap<0xdef::pool::MAIN>"
        );
    }
}
