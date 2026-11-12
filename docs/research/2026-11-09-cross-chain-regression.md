# Fresh Hoodi/Mainnet regression: 2026-11-09

Status: **small pre-spam baseline complete**. Five fixed proposals per chain passed native/root replay and fresh SP1 execute-only measurement. No repricing, guest/SDK rebuild, deployment, new timer or automatic sampling expansion. This regression supplements the [finite research-stage decision](2026-11-08-block-model-stage-closure.md).

## Selection and qualification

Mainnet chain167000 inputs47780–47784 came from the fresh Succinct production API response captured at2026-11-09T00:53:31UTC. These were the five latest fulfilled nonaggregation acquisition candidates, fixed by request time before quotations/actuals; they lagged the contemporaneous completed chain span by one proposal. Their960 blocks span00:14:47–00:46:45UTC and contain seven non-Anchor transactions.

Hoodi chain167013 proposals89370–89374 were fixed by latest completed chain spans before acquisition. Their1,910 blocks span00:18:00–00:50:01UTC, with9,527 non-Anchor transactions. Public RPC witness calls returned-32601; after the user refreshed gcloud credentials, an existing internal RPC supplied current-format preflight inputs. All five preflights validated natively; no service or deployment was changed. The temporary localhost tunnel was stopped after acquisition. The discovery CLI also returned89375 because of its inclusive endpoint expansion; that extra record is explicitly excluded by the original five-ID selection.

Existing preflight produced JSON. A host-only current GuestInput serde helper, linked to the frozen quote's exact cached type closure, serialized canonical bincode without schema translation. Its latest-Mainnet bincode→JSON→bincode qualification was byte-identical; every resulting Hoodi input then passed the quote's canonical round trip and native/root replay, followed by matching SP1 public output. No vendor decoder or new guest was involved.

Frozen identities: model `eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6`, quote SHA401a, research ELF SHA b76, SDK/executor/runner6.3.0, explicit `evm-gas-only` policy. Quote401a retains its Raiko200ccc-dirty/Reth5d-dirty build identity. The source-network Mainnet ELF is416f; its reported network PGU is metadata only. All ten actuals below are fresh local b76 measurements. No production-program/SDK parity is claimed.

## Direct new-model PGU comparison

Each row is one complete proposal. Node is the literal sum of committed integer block prices; actual is never divided into invented per-block truth.

| Chain / proposal | Blocks | Non-Anchor tx | Actual PGU | Node PGU | Full PGU | Node / actual | Full / actual | Actual minus node |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
|mainnet 47780|192|3|1,088,836,540|1,229,008,365|2,581,612,410|1.128735x|2.370983x|-140,171,825|
|mainnet 47781|192|3|1,091,924,520|1,390,107,870|2,746,422,795|1.273081x|2.515213x|-298,183,350|
|mainnet 47782|192|1|1,084,843,658|1,064,665,254|2,422,070,547|0.981400x|2.232645x|+20,178,404|
|mainnet 47783|192|0|1,075,528,114|901,943,946|2,237,379,671|0.838606x|2.080261x|+173,584,168|
|mainnet 47784|192|0|1,074,283,101|901,943,690|2,234,826,047|0.839577x|2.080295x|+172,339,411|
|hoodi 89370|381|1907|2,368,232,497|2,209,347,460|4,506,014,722|0.932910x|1.902691x|+158,885,037|
|hoodi 89371|382|1906|2,371,533,854|2,213,884,289|4,520,400,706|0.933524x|1.906108x|+157,649,565|
|hoodi 89372|382|1897|2,365,995,091|2,211,884,435|4,506,036,604|0.934864x|1.904500x|+154,110,656|
|hoodi 89373|382|1906|2,384,054,074|2,213,586,867|4,530,865,162|0.928497x|1.900488x|+170,467,207|
|hoodi 89374|383|1911|2,379,559,473|2,219,530,164|4,524,709,167|0.932748x|1.901490x|+160,029,309|

| Chain | Weighted node / actual | Node deficits | Max positive node shortfall | Weighted full / actual | Full deficits |
| --- | ---: | ---: | ---: | ---: | ---: |
|mainnet|1.013342x|3/5|173,584,168 PGU|2.256948x|0/5|
|hoodi|0.932503x|5/5|170,467,207 PGU|1.903051x|0/5|

All ten full quotes cover actual. Mainnet full/actual spans2.080261–2.515213x; Hoodi spans1.900488–1.906108x. Proposal47781 slightly crosses the earlier approximate2.5x discussion point and remains visible. Against Anchor-only47784 its operation price is479.832M PGU larger, while whole-proposal actual is17.641M larger. This is price-accounting attribution, not an isolated actual-cost diagnosis or evidence of rejection at a selected limit.

The eight positive node shortfalls remain deficits. Fixed program/startup costs stay in actual; no subtraction or full-price substitution makes node pass. These whole-proposal observations do not certify the30M-gas single-block worst case or resolve previously accepted controlled residuals.

## Comparison with current Unzen

Unzen is the original sum of canonical block difficulties, **not PGU**. Every one of the2,870 replayed Unzen block scores matches its canonical header. Workload ordering and amplification can be compared without assigning Unzen PGU semantics.

| Chain | Meter | Raw-value Pearson | Spearman | Kendall tau-b |
| --- | --- | ---: | ---: | ---: |
|mainnet|Unzen score|0.970443|0.974679|0.948683|
|mainnet|New node|0.967482|1.000000|1.000000|
|mainnet|New full proposal|0.974424|1.000000|1.000000|
|hoodi|Unzen score|0.700739|0.820783|0.737865|
|hoodi|New node|0.612010|0.600000|0.400000|
|hoodi|New full proposal|0.946366|0.900000|0.800000|

Hoodi's node ordering is weaker than Unzen in these five points (Spearman0.60 versus0.820783), while full proposal ordering is0.90. Actual spread is only0.76% in Hoodi and1.64% in Mainnet. This does not support a blanket claim that every new-model observation surface ranks workload better. Pooled Pearson is especially misleading here: Unzen reaches0.999981 largely by separating two chain clusters, despite its larger magnitude distortion.

Between these two equal-count groups, mean actual proposal workload grows2.191775x. Mean Unzen score grows4.002454x, new node2.016928x and new full1.848098x. Their scale-invariant amplification relative to actual is1.826124/0.920226/0.843197. Node is closer to this particular group-level workload change, but the groups also differ in block count and traffic; this is not independent evidence for arbitrary application scaling.

### Historical-scale diagnostic

Exactly one positive scalar per meter was fixed from the two earlier same-b76 whole-proposal observations44900/45309: `k = sum(training actual) / sum(training score)`. The same rule removes global bias from Unzen, new node and new full. Scalars are8.415935/0.607358/0.438898 respectively, with exact rational identities and raw training reports retained. No new observation was used to adjust them. This protocol was chosen after Mainnet actuals had been opened, before Hoodi actuals; Mainnet is retrospective exploratory evidence. Two training points cannot establish robust calibration or admission safety. Training quote-record identities use compact sorted JSON without a terminal newline; archive file hashes preserve the original JSONL newline.

| Chain | Unzen scaled diagnostic WAPE | New node scaled diagnostic WAPE | New full scaled diagnostic WAPE |
| --- | ---: | ---: | ---: |
|mainnet|76.09%|38.45%|6.77%|
|hoodi|221.55%|43.36%|16.48%|

These are bias-removed workload-shape diagnostics, not raw Unzen errors or new tariffs. New node/full are less distorted than Unzen under this historical scalar on this sample; the result remains conditional on the small training set. Direct new-model PGU deficits are reported only in the preceding unscaled table. Do not convert these scalars into a pricing or limit recommendation.

## Verification and retained data

All ten canonical hashes/lengths, quote/actual outputs, model/ELF/SDK/profile identities,2,870 per-block ceilings/literal sums and Unzen header bindings, full scaled totals and SDK normalization `floor(raw_gas*10/191)` were re-read. All ten estimator exits are0; no retries or replacements were used. Sequential execution used the existing6GiB/1200s guard; peak sampled process-tree PSS was4,641,510,400bytes. All40 exact receipt-owned shared-memory objects were already absent after their runs. No unrelated cleanup occurred.

The immutable [pre-spam fixture Release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-natural-cross-chain-20261109-v1) retains ten canonical inputs, original quotes/raw actuals, canonical header metadata, source provenance, standard input manifests, and the frozen diagnostic/training data. Atlas owns data; execution tools remain in Raiko. [Atlas #32](https://github.com/taikoxyz/raiko2-atlas/pull/32) carries the fixture metadata and one existing CI-matrix entry. Independent review and independent verification passed; the sole hash-scope documentation note was fixed and rechecked. A real remote download passed the unchanged CI consumer. The nonlatest asset is29,167,015bytes, SHA256 `5a3b5ddc42877e095e00e98b95d2d0f613ece28db27df0ac29fdfdf041ce0ff0`, decoding to94,801,920bytes/72 regular members. The old latest Release was preserved. GitHub PR checks are separate from this consumer readback.

## Stage decision

The small cross-chain workflow passes and supplies a pre-spam baseline without a pricing change. Completed-proposal overquotes remain approximately1.9–2.5x, and known node shortfalls remain explicit at up to173.584M PGU in these natural samples. This is consistent with the earlier finite-stage decision; it is not global block-level, production, capacity or transaction-admission certification.

Stop this bounded sampling here. After the user confirms spam activation, select a new fixed completed Hoodi interval and compare under these same frozen identities and metrics. Preserve this baseline and retain any workload growth or node deficit instead of fitting it away. No spam process, background sampler or new timer was started.
