//! Development gRPC source. No credentials are required by the public endpoint.
//! Every object read used by a frame is pinned to its effects output version.
pub mod normalize;
use crate::model::{Object, Owner};
use anyhow::{Context, Result, ensure};
use std::time::Duration;
use sui_rpc::{
    Client,
    client::HeadersInterceptor,
    field::{FieldMask, FieldMaskUtil},
    proto::sui::rpc::v2 as pb,
};

#[derive(Clone)]
pub struct Grpc {
    pub client: Client,
}
pub const DEFAULT_ENDPOINT: &str = "https://fullnode.mainnet.sui.io:443";
pub fn checkpoint_mask() -> FieldMask {
    FieldMask::from_paths([
        "sequence_number",
        "digest",
        "summary",
        "contents",
        "transactions.digest",
        "transactions.effects",
        "objects.objects.object_id",
        "objects.objects.version",
        "objects.objects.object_type",
    ])
}
impl Grpc {
    pub fn from_env() -> Result<Self> {
        let endpoint = std::env::var("SUI_GRPC_URL").unwrap_or_else(|_| DEFAULT_ENDPOINT.into());
        let mut headers = HeadersInterceptor::new();
        if let Ok(key) = std::env::var("SUI_GRPC_API_KEY")
            && !key.is_empty()
        {
            let name =
                std::env::var("SUI_GRPC_API_KEY_HEADER").unwrap_or_else(|_| "authorization".into());
            let value = if name.eq_ignore_ascii_case("authorization") {
                format!("Bearer {key}")
            } else {
                key
            };
            let mut value: tonic::metadata::MetadataValue<_> =
                value.parse().context("invalid gRPC API key header value")?;
            value.set_sensitive(true);
            let name: tonic::metadata::MetadataKey<_> =
                name.parse().context("invalid gRPC API key header name")?;
            headers.headers_mut().insert(name, value);
        }
        let client = Client::new(endpoint)
            .context("invalid gRPC endpoint")?
            .with_headers(headers)
            .with_response_headers_timeout(Duration::from_secs(15))
            .with_max_decoding_message_size(32 * 1024 * 1024);
        Ok(Self { client })
    }
    pub async fn info(&self) -> Result<pb::GetServiceInfoResponse> {
        let info = self
            .client
            .clone()
            .ledger_client()
            .get_service_info(pb::GetServiceInfoRequest::default())
            .await?
            .into_inner();
        ensure!(
            info.chain_id.as_deref() == Some(crate::config::MAINNET_CHAIN),
            "gRPC endpoint is not Sui mainnet"
        );
        Ok(info)
    }
    pub async fn checkpoint(&self, sequence: Option<u64>) -> Result<pb::Checkpoint> {
        let request = sequence
            .map(pb::GetCheckpointRequest::by_sequence_number)
            .unwrap_or_else(pb::GetCheckpointRequest::latest)
            .with_read_mask(checkpoint_mask());
        let result = tokio::time::timeout(
            Duration::from_secs(30),
            self.client.clone().ledger_client().get_checkpoint(request),
        )
        .await??
        .into_inner();
        result.checkpoint.context("gRPC checkpoint missing")
    }
    pub async fn object(&self, id: &str, version: Option<u64>) -> Result<Object> {
        let mut request = pb::GetObjectRequest::default()
            .with_object_id(id)
            .with_read_mask(FieldMask::from_paths([
                "object_id",
                "version",
                "digest",
                "owner",
                "object_type",
                "json",
            ]));
        request.version = version;
        let response = tokio::time::timeout(
            Duration::from_secs(30),
            self.client.clone().ledger_client().get_object(request),
        )
        .await??
        .into_inner();
        let object = decode(response.object.context("gRPC object missing")?)?;
        ensure!(
            object.address == id && version.is_none_or(|v| v == object.version),
            "gRPC object identity/version mismatch"
        );
        Ok(object)
    }
}
pub fn decode(object: pb::Object) -> Result<Object> {
    let mut value = serde_json::to_value(&object)?;
    let contents = value
        .get_mut("json")
        .context("gRPC Move JSON missing; provider must support JSON rendering")?
        .take();
    let owner = object.owner.context("gRPC owner missing")?;
    let kind = match owner.kind() {
        pb::owner::OwnerKind::Address => "AddressOwner",
        pb::owner::OwnerKind::Object => "ObjectOwner",
        pb::owner::OwnerKind::Shared => "Shared",
        pb::owner::OwnerKind::Immutable => "Immutable",
        pb::owner::OwnerKind::ConsensusAddress => "ConsensusAddressOwner",
        _ => anyhow::bail!("unsupported gRPC owner kind"),
    };
    let object = Object {
        address: object.object_id.context("missing object ID")?,
        version: object.version.context("missing object version")?,
        digest: object.digest.context("missing object digest")?,
        move_type: object.object_type.context("missing object type")?,
        contents,
        owner: Some(Owner {
            kind: kind.into(),
            address: owner.address,
        }),
    };
    object.validate()?;
    Ok(object)
}

impl Grpc {
    pub fn resume(
        &self,
        start: u64,
    ) -> impl futures_util::Stream<
        Item = std::result::Result<sui_rpc::client::CheckpointStreamFrame, tonic::Status>,
    > + Send
    + 'static {
        use sui_rpc::client::{CheckpointStreamRequest, CheckpointStreamStart, LedgerStreamConfig};
        let request = CheckpointStreamRequest::new()
            .with_start(CheckpointStreamStart::Checkpoint(start))
            .with_read_mask(checkpoint_mask());
        let mut limits = LedgerStreamConfig::default();
        limits.max_buffered_live_items = std::num::NonZeroUsize::new(2).unwrap();
        limits.list_page_limit = Some(1);
        self.client.stream_checkpoints_with_config(request, limits)
    }
}
