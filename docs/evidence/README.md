# Research evidence

The original research files below are public mainnet observations made on 2026-10-19, not a complete
normalized State, runtime fixtures, or a passing verification sample. Requests
were made at different times. Preserve the distinction between a checkpoint
timestamp, an object's version, and cached price/interest timestamps.

* `mainnet-market-2026-10-19.json`: unmodified GraphQL response with all 45
  reserves, the main market, and UpgradeCap. See the adjacent request file.
* `grpc-discovery-2026-10-19.json`: one page of two dynamic-object fields.
  Its continuation token means there are more obligations; this is not a census.
* `grpc-batch-request.json`: exact batch request generated from those child IDs.
* `grpc-obligations-2026-10-19.json`: batch response with both real obligations.
  Both have zero weighted debt; they are transport evidence, not HF test cases.
* `reserve-feeds.json`: derived inventory of raw coin/feed associations from the
  market response. Null price-object IDs are unresolved, not absent onchain.
* `provenance.json`: source revision, published SDK integrity, and SHA-256 hashes.

Reproduce the GraphQL research read (the resulting state will have changed):

```sh
curl --fail --silent --show-error --max-time 30 \
  https://graphql.mainnet.sui.io/graphql \
  -H 'Content-Type: application/json' \
  --data-binary @docs/evidence/graphql-market-request.json
```

Reproduce the first discovery page, then batch-read the captured IDs:

```sh
grpcurl -max-time 20 \
  -d '{"parent":"0xcffebc0eb9d701d7843346dab87a043a60e28e595278db5764b87a4b3919a00a","page_size":2,"read_mask":{"paths":["kind","parent","field_id","child_id","value_type"]}}' \
  fullnode.mainnet.sui.io:443 sui.rpc.v2.StateService/ListDynamicFields

grpcurl -max-time 20 -d @ fullnode.mainnet.sui.io:443 \
  sui.rpc.v2.LedgerService/BatchGetObjects \
  < docs/evidence/grpc-batch-request.json
```

These are manual research probes, not the intended production ingestion/retry
implementation. No API keys, wallet material, or authenticated request headers
are included. The parent table ID is an observation; runtime must discover it
from the market. The discovery result's JSON `childId` corresponds to the
protobuf field `child_id`. `dynamic_object_id` is rejected by this live server.

## Implemented full census

`snapshot-2026-10-19.json` records the later completed census at checkpoint
335839129: 159,717 obligations, 155,517 visible caps, and 26 market type instances.
It includes per-market counts and SHA-256 hashes of the local raw artifacts and
reviewed reports. The 512 MB raw capture is intentionally excluded from git.
All cached diagnostics were computed; fresh valuation rules remain unverified.
The full census found multiple caps for ten obligations, which the reviewed
report preserves as ambiguous ownership. See the [code review](../reviews/2026-10-19.md).
