//! Streaming census. Each page and object is read under the same checkpoint scope.

use anyhow::{Context, Result, ensure};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::task::JoinSet;

use crate::{
    config::{Config, MAINNET_CHAIN},
    ingest::graphql::{Graphql, Pagination},
    model::{Capture, Checkpoint, Manifest, Market, integer, string},
    storage,
};

pub struct Options {
    pub checkpoint: Option<u64>,
    pub markets: Vec<String>,
    pub all_markets: bool,
    pub page_size: u32,
    /// Explicit sampling only. A truncated census is always marked incomplete.
    pub max_pages: Option<u64>,
    pub out: PathBuf,
}

pub async fn discover(
    client: &Graphql,
    config: &Config,
    checkpoint: &Checkpoint,
    page_size: u32,
) -> Result<Vec<Market>> {
    let mut pagination = Pagination::default();
    let mut markets = Vec::new();
    let package = &config.suilend.original_package_id;
    let market_type_filter = crate::protocols::suilend::market_type_filter(package);
    loop {
        let page = client
            .objects(
                checkpoint.sequence_number,
                &market_type_filter,
                pagination.cursor.as_deref(),
                page_size,
            )
            .await?;
        pagination.accept(&page)?;
        for object in page.objects {
            markets.push(Market::new(object, package)?);
        }
        if pagination.cursor.is_none() {
            break;
        }
    }
    markets.sort_by(|a, b| a.object.address.cmp(&b.object.address));
    Ok(markets)
}

pub async fn capture(client: Graphql, config: &Config, options: Options) -> Result<Manifest> {
    ensure!(
        (1..=50).contains(&options.page_size),
        "page size must be 1..=50"
    );
    ensure!(options.max_pages != Some(0), "max-pages must be positive");
    ensure!(
        !options.all_markets || options.markets.is_empty(),
        "choose explicit markets or all-markets"
    );
    let checkpoint = client.checkpoint(options.checkpoint).await?;
    let (upgrade_cap, clock) = tokio::try_join!(
        client.object(checkpoint.sequence_number, &config.suilend.upgrade_cap_id),
        client.object(
            checkpoint.sequence_number,
            "0x0000000000000000000000000000000000000000000000000000000000000006"
        ),
    )?;
    ensure!(
        integer(&upgrade_cap.contents, "version")? == config.suilend.package_version
            && string(&upgrade_cap.contents, "package")? == config.suilend.published_package_id,
        "deployment changed: update and review configured package identity"
    );
    integer(&clock.contents, "timestamp_ms")?;
    let mut markets = discover(&client, config, &checkpoint, options.page_size).await?;
    if !options.all_markets {
        let ids = if options.markets.is_empty() {
            vec![config.suilend.lending_market_id.clone()]
        } else {
            options.markets.clone()
        };
        for id in &ids {
            ensure!(
                markets.iter().any(|m| &m.object.address == id),
                "market {id} not discovered"
            );
        }
        markets.retain(|m| ids.contains(&m.object.address));
    }
    ensure!(!markets.is_empty(), "no markets selected");
    for market in &markets {
        if market.object.address == config.suilend.lending_market_id {
            ensure!(
                market.type_argument == config.suilend.lending_market_type,
                "main market type changed"
            );
            ensure!(
                integer(&market.object.contents, "version")? == config.suilend.market_version,
                "main market schema version changed"
            );
        }
    }
    storage::create_directory(&options.out)?;
    let out = Arc::new(options.out);
    let mut tasks = JoinSet::new();
    for market in markets {
        let client = client.clone();
        let out = out.clone();
        let package = config.suilend.original_package_id.clone();
        let cp = checkpoint.sequence_number;
        tasks.spawn(async move {
            let collector = Collector {
                client: &client,
                checkpoint: cp,
                market: &market,
                package: &package,
                out: &out,
                page_size: options.page_size,
                max_pages: options.max_pages,
            };
            let (obligations, caps) = tokio::try_join!(
                collector.collect(CollectionKind::Obligations),
                collector.collect(CollectionKind::OwnerCaps),
            )?;
            Ok::<_, anyhow::Error>(Capture {
                market,
                obligations: obligations.0,
                obligations_exhausted: obligations.1,
                owner_caps: caps.0,
                owner_caps_exhausted: caps.1,
            })
        });
    }
    let mut captures = Vec::new();
    while let Some(result) = tasks.join_next().await {
        captures.push(result.context("capture task failed")??);
    }
    captures.sort_by(|a, b| a.market.object.address.cmp(&b.market.object.address));
    let complete = captures.iter().all(|c| {
        c.obligations_exhausted
            && c.owner_caps_exhausted
            && c.obligations == c.market.expected_obligations
    });
    let manifest = Manifest {
        schema_version: 2,
        chain_identifier: MAINNET_CHAIN.to_owned(),
        checkpoint,
        original_package_id: config.suilend.original_package_id.clone(),
        upgrade_cap,
        clock,
        scope: if options.all_markets {
            "all_discovered_type_instances_not_an_endorsement_registry"
        } else {
            "explicit_markets"
        }
        .to_owned(),
        complete,
        markets: captures,
    };
    storage::json(&out.join("capture-config.json"), config)?;
    storage::json(&out.join("manifest.json"), &manifest)?;
    crate::storage::raw::seal(&out, &manifest)?;
    Ok(manifest)
}

enum CollectionKind {
    Obligations,
    OwnerCaps,
}

struct Collector<'a> {
    client: &'a Graphql,
    checkpoint: u64,
    market: &'a Market,
    package: &'a str,
    out: &'a Path,
    page_size: u32,
    max_pages: Option<u64>,
}

impl Collector<'_> {
    async fn collect(&self, collection: CollectionKind) -> Result<(u64, bool)> {
        let Self {
            client,
            checkpoint,
            market,
            package,
            out,
            page_size: size,
            max_pages,
        } = *self;
        let caps = matches!(collection, CollectionKind::OwnerCaps);
        let kind = if caps { "caps" } else { "obligations" };
        let cap_type = market.cap_type(package);
        let mut output = storage::writer(&storage::market_file(out, &market.object.address, kind))?;
        let mut pagination = Pagination::default();
        loop {
            let page = if caps {
                client
                    .objects(checkpoint, &cap_type, pagination.cursor.as_deref(), size)
                    .await?
            } else {
                client
                    .obligations(
                        checkpoint,
                        &market.table_id,
                        pagination.cursor.as_deref(),
                        size,
                    )
                    .await?
            };
            pagination.accept(&page)?;
            for object in page.objects {
                if caps {
                    ensure!(object.move_type == cap_type, "owner cap type mismatch");
                    string(&object.contents, "obligation_id")?;
                } else {
                    market.validate_obligation(&object, package)?;
                }
                storage::line(&mut output, &object)?;
            }
            if !caps {
                ensure!(
                    pagination.count() <= market.expected_obligations,
                    "more obligations than table size"
                );
            }
            if pagination.pages.is_multiple_of(100) {
                eprintln!(
                    "{} {kind}: {} objects at checkpoint {checkpoint}",
                    market.object.address,
                    pagination.count()
                );
            }
            if pagination.cursor.is_none()
                || max_pages.is_some_and(|limit| pagination.pages >= limit)
            {
                break;
            }
        }
        let exhausted = pagination.cursor.is_none();
        if !caps && exhausted {
            ensure!(
                pagination.count() == market.expected_obligations,
                "obligation census does not match table size"
            );
        }
        output.flush()?;
        output.get_ref().sync_all()?;
        eprintln!(
            "{} {kind}: {} objects, exhausted={exhausted}",
            market.object.address,
            pagination.count()
        );
        Ok((pagination.count(), exhausted))
    }
}
