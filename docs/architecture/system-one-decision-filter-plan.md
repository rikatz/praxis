# System One decision filter: implementation plan

Status: MVP implementation is in progress. The `system_one_decision` filter now validates configuration, captures configured request state, sends one bounded System One request through Praxis transport, validates the complete response against configured questions, and stores a typed request-scoped answer. It does not publish answers to PPE, routing, headers, or branch results. **Part I / Phase 1** is the MVP: one System One request and a validated, request-scoped answer. **Part II / Phase 2** retains the deferred policy, routing, local deployment, and operational plan. Use `jev` for its typed response model and the Praxis JSON request envelope for nested state. Tests and other checks are omitted at the user's request.

## 1. Context and boundaries

[TypeSafe's System One API](https://docs.typesafe.ai/api) evaluates a JSON-compatible `state` against a map of independent `questions` in one `POST /v1/systemone` call. A `noul` answer is the probability of yes; `choice` selects a configured option and returns a distribution; `score` returns an expected position over an ordered rubric and a distribution. Questions and their instructions are static Praxis configuration. Request data supplies state, never question text or endpoint selection.

| Backend | Relationship to System One | Hosting |
| --- | --- | --- |
| [Jev](https://docs.typesafe.ai/models) | TypeSafe's reference System One model and hosted API | TypeSafe |
| [Kev](https://github.com/jaredpalmer/kev) | Independent Qwen-derived model with a decision head and compatible `/v1/systemone` server | Self-hosted through Kev's own server |
| [Laya](https://github.com/NandhaKishorM/laya) | Independent encoder model with a decision head and compatible core API; limits and confidence behavior differ, and unknown model names may select an automatic route | Self-hosted |
| [vLLM structured diffusion](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/) | A `structured_server.py` wrapper provides the System One endpoint for DiffusionGemma | Self-hosted vLLM plus wrapper |

These are interchangeable at the **wire boundary**, not equivalent models. Kev is not served by the linked vLLM example; that example serves DiffusionGemma. The returned `model` name may be an alias or wrapper label, so Phase 2 records the deployed checkpoint separately. Thresholds must be calibrated for each model, version, and question set. The public API does not establish Jev's internal architecture.

Use cases from [TypeSafe's use-case map](https://docs.typesafe.ai/concepts/use-case-map), applied to Praxis:

| Case | Example configured state | Question and consumer |
| --- | --- | --- |
| Smart model routing | Prompt/messages and static cost or latency preference | `choice` of allowed model tiers; existing router selects the cluster |
| Malicious HTTP traffic | Method, path, query, bounded body | `noul` for suspected abuse and `score` for severity; PPE decides allow/review/deny |
| LLM input guardrails | Messages and static policy excerpt | `noul` for jailbreak or data leakage; PPE input rule |
| Tool-call verification | Tool name and arguments | `noul` for unsafe call; PPE tool rule |
| Support triage | Ticket text and tenant tier | `choice` of queue; router or branch chain |
| RAG context selection | Query and candidate excerpt | `score` for relevance; application or PPE |
| Marketplace moderation | Listing text and rules | `noul` for prohibited content; PPE review/deny |
| Lead or incident triage | Message or report plus fixed criteria | `noul` for intent or suspicious signals; `choice` for category; branch or upstream service |

The model supplies a judgment, not proof of safety. Request headers, query, and body are untrusted model input. Existing authentication, ACLs, rate limits, router, branch chains, and PPE retain their responsibilities. This release does not inspect image, audio, video, or binary payloads; reject them when a required security question depends on their content. The filter is request-side only; response guardrails are later work.

### Rust `jev` crate and the JSON gap

The [System One API](https://docs.typesafe.ai/api) accepts JSON state. The
[published Rust `jev` crate](https://docs.rs/jev/latest/jev/) (0.1.2) narrows
`State` to text or `{text, facts}` with string facts; it cannot represent the
configured nested header map through its public state type. Its `Question`
type supports the string instructions and criteria used by this MVP. These
are crate limits, not model or HTTP protocol limits.

The crate has no supported public request type accepting Praxis's JSON state
and questions together. Use `jev::Evaluation` and `jev::Answer` to decode
typed answers, then apply Praxis validation before publishing. Build the
minimal request envelope
`{model, state, questions}` with `serde_json` from validated config and the
request state, then send it through Praxis's existing bounded
`execute_subrequest`. This is the local bridge needed to preserve the API's
JSON input and avoid a second HTTP client or custom response parser. Do not depend on the crate's
`#[doc(hidden)]` request/parse helpers; its public `Evaluation` implements
`Deserialize`.

Pin `jev` 0.1.2 in the workspace with `default-features = false`; the
`async` feature and the crate's HTTP client are unnecessary for this
integration. Its default feature selects a separate AWS-LC TLS provider,
whereas Praxis already owns outbound TLS and transport. The crate's
`Evaluation` parser accepts tagged and untagged backend answers but defaults
some missing metadata; it does **not** establish that an answer is safe or
matches the configured question. Preserve the strict validation below.
Review the dependency graph for Praxis's Rust 1.92 and FIPS builds when
adding the dependency: `jev` 0.1.2 still declares Reqwest as a non-optional
dependency even when its default features and HTTP client use are disabled.

### Praxis seams to reuse

- Add a built-in `HttpFilter` under `crates/filter/src/builtins/http/security/` and register it in `crates/filter/src/registry.rs`. Header-only mappings run in `on_request`; a configured `request.body` mapping waits for the complete body and evaluates in the request-body hook.
- `HttpFilterContext::execute_subrequest` provides the outbound request path. It requires an explicit deadline and response cap even before those become configurable. The crate's `AsyncTypeSafeClient` owns a private Reqwest client with automatic 429/529 retries and no exposed response-byte cap or Praxis connector injection, so the filter does not call it.
- `ctx.request.headers` is the request header map after Praxis's host/header normalization and reserved-header validation. Client-supplied `x-praxis-*`/`x-ext-*` headers are rejected before filters run. Capture it at the request hook where the configured state is completed.
- `RequestExtensions` can hold a typed, request-scoped result. It has one slot per Rust type, so allow one instance of this filter per pipeline. `FilterResultSet` holds only short strings keyed by filter type for branches and cannot hold the full answer.
- PPE builds fresh `Extensions` for each hook. Bridge only the validated typed decision; generic `structured_metadata` is not authoritative because `json_body` can populate it from client content. Security-class filters cannot use request `conditions` under the current pipeline rules; scope this filter by listener or chain placement. Keep the built-in independent of the optional `policy-engine` feature.
- Later, the existing `router`, `json_body`, branch results, and PPE `policy` filter consume the typed result. No second router or backend-specific HTTP client is needed.

## 2. Phase 1 — MVP: query and receive an answer

### Scope and configuration

Build one request-side filter that sends **one** System One call per request and stores its answer. Configuration has `endpoint`, `model`, optional `api_key_env`, a nonempty `state` mapping, and a nonempty `questions` map. `state` maps arbitrary nonempty output keys either to an arbitrary static string or to a supported Praxis context source. Header values are included only when configured; mapping `headers: {from: request.headers}` passes the complete post-sanitization header map. The MVP has no output publication, threshold, policy rule, router integration, configurable timeout/body limits, or fallback mode.

Proposed filter-chain fragment:

```yaml
filter_chains:
  - name: main
    filters:
      - filter: system_one_decision
        endpoint: https://api.typesafe.ai/v1/systemone
        model: jev-1.13.0
        api_key_env: TYPESAFE_API_KEY
        state:
          headers: {from: request.headers}
          method: {from: request.method}
          path: {from: request.path}
          purpose: "Classify this request for automated abuse."
        questions:
          automated_abuse:
            type: noul
            instructions: "Do these request headers suggest automated abuse? Use the state headers."
      - filter: router
        routes:
          - path_prefix: /
            cluster: default
```

This is a proposed fragment, not a runnable current configuration. A full example must add a listener and a load balancer. For hosted TypeSafe, `api_key_env` is required and names an environment variable; the value never goes in YAML. The scaffold accepts an absolute `http` or `https` URL with a host. Before transport is implemented, require `/v1/systemone`, reject URL userinfo/query/fragment and redirects, and restrict hosted MVP calls to HTTPS. Validate the address actually dialed and deny private, loopback, link-local, and metadata destinations. Local HTTP endpoints and explicit address opt-ins are Phase 2. Reject unknown configuration fields.

Support `noul`, `choice`, and `score` questions. `instructions` is a nonblank string. `noul.criteria` may contain nonblank `true` and/or `false` strings; `choice.criteria` maps nonblank option labels to optional nonblank string descriptions; `score.criteria` is an ordered list of at least two nonblank strings. Require nonblank question IDs; do not add an identifier regex or length restriction unless a backend requires one. Keep questions fixed during a request. These string-valued shapes match `jev::Question` while the local request envelope preserves the nested JSON state.

### Exact MVP state

Build one JSON object from the configured output keys. A static string is sent unchanged; a source mapping is resolved from the request context. To include every post-sanitization header, configure `headers` as `request.headers`:

```json
{
  "headers": {
    "host": ["app.example"],
    "authorization": ["Bearer client-token"],
    "cookie": ["session=abc"],
    "x-tag": ["first", "second"]
  },
  "method": "POST",
  "path": "/v1/chat/completions",
  "purpose": "Classify this request for automated abuse."
}
```

`request.headers` resolves to a nested object with lowercase header names and **arrays of strings for every header**, including single values. Preserve repeated values in received order and empty values as `""`. If a header value cannot be represented as a string, fail closed; do not invent a base64 header encoding or silently omit it. Praxis strips reserved client `x-praxis-*`/`x-ext-*` headers before this filter. MVP sources resolve to their natural JSON values: method, path, raw query, trusted client IP, and `request.body` as the complete body string. `HttpFilterContext` does not expose HTTP version, client port, or the accepted server socket address. Pingora stores the HTTP version on its own request context for Via handling, but that value is not currently passed to filters. HTTP/2 pseudo-headers are represented through explicit sources, not as entries in `HeaderMap`.

When `request.headers` is selected, this intentionally sends `Authorization`, `Cookie`, and every other post-sanitization header **inside JSON state** to the configured decision endpoint. They are not copied into the outbound HTTP header section; outbound authorization comes only from `api_key_env`. Operators must choose an endpoint permitted to receive these values. If the serialized request exceeds the fixed MVP bound, reject the request rather than send partial state.

The map is the post-validation inbound request snapshot, independent of later filter mutations. Place this filter early when that snapshot should drive the decision.

### Wire request, answer, and failures

Build the following three-field JSON envelope locally and send it as one `POST /v1/systemone` through `execute_subrequest`. `questions` comes from validated static config; `state` is built per request. The crate has no supported public request type that accepts both inputs.

```json
{
  "model": "jev-1.13.0",
  "state": {"headers": {"host": ["app.example"], "x-tag": ["first", "second"]}, "method": "POST", "path": "/v1/chat/completions", "purpose": "Classify this request for automated abuse."},
  "questions": {
    "automated_abuse": {
      "type": "noul",
      "instructions": "Do these request headers suggest automated abuse? Use the state headers."
    }
  }
}
```

Accept a successful response only after validating its structure against the configured questions. Decode it as `jev::Evaluation` and store it in a crate-private typed `SystemOneDecision` in `ctx.extensions` with the requested model and presence information for optional `reported_model` and `usage`. The resulting request-scoped value has this conceptual shape:

```json
{
  "requested_model": "jev-1.13.0",
  "reported_model": "jev-1.13.0",
  "answers": {
    "automated_abuse": {"type": "noul", "noul": 0.31}
  },
  "usage": {"input_tokens": 42, "output_tokens": 8}
}
```

This is the **Praxis request-scoped result**, not a new field claimed to exist in the TypeSafe response. The API response uses `model`, `answers`, and `usage`; compatible self-hosted servers may omit usage. Consumers can read the typed extension later; the MVP does not yet translate it into PPE attributes, branch strings, or headers.

Inspect the bounded raw JSON for required field presence, then deserialize into `jev::Evaluation`. Require an `answers` object and all type-specific answer fields. The MVP has no hosted/self-hosted backend setting, so `model` may be omitted for local servers; if present it must be a nonempty string. Omit `reported_model` or `usage` from the Praxis result when absent, rather than reporting the crate's default empty model or zero usage. The crate tolerates untagged answers and defaults some missing fields to empty or zero; Praxis must not treat a defaulted field as an observed answer. Validate exactly one answer for each configured ID, no unknown IDs, and matching answer types. Require `noul` and each probability/confidence to be finite and in `[0, 1]`. For `choice`, require a configured choice, probability keys equal to configured options, a distribution sum within 0.02 of 1, and a chosen option with maximal probability (ties allowed). For `score`, require the numeric rubric legend (`"0"` through `"n-1"`) to match the configured ordered criteria, probability keys equal to those rubric indices, the same distribution bounds, and a finite score in `[0, level_count - 1]` within 0.02 of the probability-weighted level. Usage, if present, must be an object with nonnegative integer `input_tokens` and `output_tokens`. Permit extra top-level backend diagnostics, but do not expose them as stable decision fields. Publish nothing from a partial or malformed answer.

Use existing subrequest plumbing with bounded limits and no retry. The complete request body has a fixed 32 KiB cap when `request.body` is selected. `request_size_limit_bytes`, `response_size_limit_bytes`, and `request_timeout_ms` are validated configuration fields with defaults of 128 KiB, 64 KiB, and 2 seconds, and maxima of 128 KiB, 64 KiB, and 10 seconds. Reject invalid text header/body values with 400, unavailable capture context with 500, and an oversized body or serialized request with 413. Timeout/network/capacity and upstream 429/529/503 map to 503; unexpected status, upstream 401/422, oversized response, malformed JSON, or invalid answer map to 502. Fail closed with an explicit rejection and publish no result. Keep `failure_mode: closed`; do not add an MVP fallback.

### Manual implementation steps — Phase 1

1. **Scaffolded:** `jev` 0.1.2, the security filter module, registry entry, and build-time check rejecting multiple instances across main and branch filters. `RequestExtensions` has one slot per type, so preserve this invariant.
2. **Scaffolded:** parse and validate `endpoint`, `model`, optional `api_key_env`, `state`, and `questions`; resolve the key at build/reload. The current config validates nonempty state keys, supported source names, and string-valued question shapes. Complete endpoint transport restrictions in the request path before sending.
3. **Implemented:** resolve only configured state entries, preserving all post-sanitization header values as lowercase-name to array-of-strings. MVP source mappings are `request.method`, `request.path`, `request.query`, `request.headers`, `request.body`, and `client.ip`. Invalid header/body text rejects with 400; unavailable context rejects with 500. Body-dependent state uses read-only `StreamBuffer`, enforces the fixed 32 KiB limit during pre-read, and is captured after the complete body is available.
4. **Implemented:** serialize the `{model,state,questions}` envelope once with `serde_json`, enforce configured request/response limits and timeout, and call `execute_subrequest` once. Send incoming headers only inside state; do not construct `jev::AsyncTypeSafeClient`.
5. **Implemented:** inspect the bounded response JSON for required fields, deserialize with `jev::Evaluation`, and validate the complete answer set against the configured questions. Store the typed `SystemOneDecision` in `ctx.extensions` only on success. Reject input, transport, HTTP, and answer failures as specified above.
6. Add a complete Jev example config under `examples/configs/security/` and document this MVP filter contract. The example should show that the answer is request-scoped and currently has no routing or PPE effect.

**MVP exit condition:** one configured question reaches System One, each configured state mapping is present in the JSON state (including every post-sanitization header when `request.headers` is mapped), and Praxis stores the complete validated answer for the same request.

## 3. Part II / Phase 2 — deferred full implementation plan

Phase 2 builds on the MVP result. Nothing in this part is required for the request-scoped answer MVP. Add only the fields needed for a concrete consuming use case; these are **deferred from Phase 1**:

| Deferred field or capability | Phase 2 purpose |
| --- | --- |
| JSON selectors, `max_body_bytes`, `evaluate_in_pre_read` | Add JSON Pointer selection from a JSON body, operator-controlled body bounds, and a header-only pre-read decision when an earlier body-phase PPE hook must consume it. MVP `request.body` uses a fixed internal cap and waits for the complete body. |
| `max_in_flight`, `max_state_bytes` | Add concurrency limiting and a separate serialized-state cap if request size limits prove too coarse. MVP already has validated request-size, response-size, and timeout controls. |
| `allow_private_endpoint`, `allow_loopback_endpoint` | Permit a fixed self-hosted endpoint after checking every dialed address. Loopback is an exact literal `127.0.0.1` or `[::1]` opt-in; never allow link-local or cloud metadata addresses. |
| `backend`, `checkpoint`, `question_set_id` | Record deployment and question-version identity for calibration, metrics, and PPE. Do not infer checkpoint identity from the response `model`. |
| `role` | `security` for any enforcing question, including a filter that also routes; `routing_only` only when no enforcing policy consumes its answers. |
| `request.protocol`, `client.port`, `server.ip`, `server.port` state sources | Expose HTTP version and trusted connection/listener socket metadata. The HTTP version already exists on `PingoraRequestCtx` but is not available in `HttpFilterContext`; client port and server socket address are not currently forwarded to filters. Add only the required context fields and initialize synthetic contexts appropriately. |
| `publish` | Map validated, allowed answer values to reserved `x-praxis-*` headers or short branch-result strings. No arbitrary model-generated headers, paths, URLs, or clusters. |
| `policy.require_system_one` and PPE bridge | Require the typed result and expose a stable `custom.system_one` object to policy rules. |

These are Phase 2 fields, not placeholders to implement in Phase 1. Keep the built-in security-class registration and Praxis `failure_mode: closed` for both roles.

### Deferred configuration contract

- Keep the Phase 1 `endpoint`, `model`, optional `api_key_env`, direct `state` mapping, and `questions` fields. Questions remain static on each built pipeline and use string instructions and criteria. Reload rebuilds the filter, validates every field, and swaps it with the pipeline; no request uses half of an old and half of a new question set.
- Preserve the direct `state` map: each arbitrary output key has either a static string or a Praxis source. No source is implicit; map `headers: {from: request.headers}` to include all headers. Phase 2 may extend a source mapping with explicit JSON-body selectors such as a JSON Pointer and required/optional behavior. Reject duplicate output keys and oversized configured state at build/reload. Avoid credentials in static state.
- Require `timeout_ms` in Phase 2 as the whole decision call deadline, including connection setup and reading the bounded response. `max_in_flight` limits concurrent calls per filter instance; exhaustion is a capacity failure. `max_state_bytes` caps serialized state, `max_request_bytes` caps the complete envelope, `max_response_bytes` caps the HTTP response, and `max_body_bytes` caps body capture. Validate positive, finite upper bounds during build/reload. Do not add retries or partial-state fallback.
- Require a finite `max_body_bytes` whenever body state or `evaluate_in_pre_read: true` activates body buffering. Header-only calls that run solely in `on_request` keep streaming request bodies untouched.
- `allow_private_endpoint` applies only to the configured host and RFC 1918/unique-local addresses. `allow_loopback_endpoint` applies only to an exact loopback IP literal. Resolve and validate the destination used for every connection, including after reload; deny redirects and all link-local, metadata, multicast, and unspecified destinations. Keep certificate verification for HTTPS. An HTTP local endpoint requires an explicit address opt-in.
- `role` defaults to `security`, and that value is required when any answer can affect allow, deny, review, or a mandatory guardrail. A combined routing and security filter also has this role. `role: routing_only` permits only an explicit deterministic router fallback when the backend is unavailable. The role governs failure behavior; it does not change answer parsing or allow an uncertain answer to count as safe.
- Keep `api_key_env` required for hosted TypeSafe and optional for explicitly local/private endpoints. Resolve the named variable at build/reload and reject an empty value; never print the secret in diagnostics. Pin the configured production model version where possible.
- `backend`, `checkpoint`, and `question_set_id` are operator-supplied stable identifiers; require them in Phase 2 and cap each at 64 ASCII bytes. Change `question_set_id` whenever question wording or criteria change. Keep the configured `model` and the optional backend-reported `model` separate. Use these identifiers to select calibrated thresholds and to label bounded metrics; never rely on a reported alias to identify model weights.
- `publish` is optional. Each entry names a configured question and a single fixed output target. It may publish only operator-enumerated labels or choice options, with a validated confidence/probability gate. Keep the full answer in `SystemOneDecision`; a header or branch string is a deliberately small projection.

Validate each `publish` rule against a configured question and answer field. A choice header can contain only a configured `allowed_values` entry when that option's probability meets `min_probability`; otherwise omit it and use the router fallback. A `noul` or `score` rule may emit only configured short verdict labels above or below its threshold. Reject duplicate output targets, invalid thresholds, unsafe header values, or mismatched question types at build/reload. A below-threshold verdict means only "threshold not met," never "safe."

For the portable Kev/Laya/vLLM examples, keep at most 64 questions, 2–26 choice options per question, 2–10 score levels, and no more than 512 combined options; Laya also has a 50,000-character state limit. These are compatibility ceilings, not useful defaults. The TypeSafe API may support richer question JSON, but this plan deliberately uses the string subset accepted by `jev` and compatible local backends. Reject a backend-specific incompatible configuration rather than alter questions at runtime. Validate configured question text against the request cap before the filter starts.

### Phase 2 state and answer contract

Build the state object only from configured mappings. MVP sources are `request.method`, `request.path`, `request.query`, `request.headers`, `request.body`, and `client.ip`. Phase 2 may add `request.protocol`, `client.port`, `server.ip`, and `server.port`; none is currently available through `HttpFilterContext`. A missing source value or non-text header/body value fails closed; do not send a partial state. `request.query` is the raw query string without the leading `?`; `request.path` excludes the query. `request.headers` includes every post-sanitization header as lowercase name to array of strings. Use the trusted connection for client IP, not `X-Forwarded-For`. Phase 2 may add explicit body JSON selectors: a JSON Pointer can preserve an object's, array's, scalar's, or null value. Require UTF-8 JSON media types for those selectors; reject unsupported encoding/media type with 415, malformed JSON with 400, and oversized body/state/request with 413. Never truncate content used for a security decision. For OpenAI-style messages, require text-only `message.content`; binary/multimodal input needs another control.

Source behavior to implement:

| Source | Value in `state` | When evaluated |
| --- | --- | --- |
| `request.method`, `request.path`, `request.query`, `client.ip` | Method/path/raw query or trusted client IP. Path excludes query; query has no leading `?`. | `on_request` |
| `request.headers` | Complete post-sanitization map: lowercase header name to an array of strings, including one-element arrays. | `on_request` |
| `request.body` | Complete UTF-8 request body as a string; fail closed on unsupported/binary content or fixed MVP size limit. | End of request body |
| `request.protocol` (Phase 2) | HTTP version from `PingoraRequestCtx`; it is not passed through to `HttpFilterContext` today. | `on_request` |
| `client.port`, `server.ip`, `server.port` (Phase 2) | Trusted connection/listener metadata; ports are JSON integers. Requires context plumbing not present in `HttpFilterContext` today. | `on_request` |
| Phase 2 JSON body selector | JSON value at a configured JSON Pointer, preserving object, array, scalar, or null. | End of request body |

For example, the combined configuration below explicitly maps the full header map and other request fields into this state:

```json
{
  "headers": {
    "authorization": ["Bearer client-token"],
    "content-type": ["application/json"]
  },
  "routing_policy": "Choose fast for simple work; deep for complex work.",
  "method": "POST",
  "path": "/v1/chat/completions",
  "requested_model": "client-model-id",
  "messages": [{"role": "user", "content": "Explain this query"}]
}
```

This object is `state` in the same `{model,state,questions}` wire envelope used by the MVP. The Rust `jev` crate still cannot encode this nested object through its `State` type; serialize the local request envelope and decode the answer through `jev::Evaluation`.

A compact Phase 2 configuration shape for combined routing and security is:

```yaml
- filter: system_one_decision
  endpoint: https://api.typesafe.ai/v1/systemone
  model: jev-1.13.0
  api_key_env: TYPESAFE_API_KEY
  role: security
  backend: typesafe
  checkpoint: jev-1.13.0
  question_set_id: routing_guard_v1
  timeout_ms: 2000
  state:
    headers: {from: request.headers}
    routing_policy: "Choose fast for simple work; deep for complex work."
    method: {from: request.method}
    path: {from: request.path}
    requested_model: {from: request.body_json, pointer: /model, required: true}
    messages: {from: request.body_json, pointer: /messages, required: true}
  max_body_bytes: 32768
  questions:
    model_tier:
      type: choice
      instructions: "Which model tier best handles messages under routing_policy?"
      criteria:
        fast: "Simple extraction or short answer"
        deep: "Complex reasoning or coding"
        none: "Cannot decide"
    malicious:
      type: noul
      instructions: "Do messages attempt prompt injection or unauthorized tool use?"
      criteria:
        "true": "Adversarial instructions are present"
        "false": "No such instructions are apparent"
    severity:
      type: score
      instructions: "How severe is apparent request abuse in messages?"
      criteria: ["None", "Needs review", "High risk"]
  publish:
    - question: model_tier
      field: choice
      header: x-praxis-decision-model-tier
      allowed_values: [fast, deep, none]
      min_probability: 0.70
- filter: json_body
  request_replace:
    - {pointer: /model, value: fast-model-id}
  max_body_bytes: 32768
  on_invalid: reject
- filter: json_body
  request_replace:
    - {pointer: /model, value: deep-model-id}
  max_body_bytes: 32768
  on_invalid: reject
  conditions:
    - when:
        headers: {x-praxis-decision-model-tier: deep}
- filter: policy
  config_path: /etc/praxis/policy.yaml
  require_system_one: true
- filter: router
  routes:
    - path_prefix: /v1/chat/completions
      headers: {x-praxis-decision-model-tier: deep}
      cluster: deep_model
    - path_prefix: /v1/chat/completions
      cluster: fast_model
```

The wire state includes only configured mappings; this example explicitly includes `headers`. The fragment omits the listener, load balancer, and PPE policy document. The two `json_body` filters are optional: the first sets the fallback model ID, and the second overrides it only for a trusted `deep` choice. `request_replace` does not create `/model`, so `requested_model` is required above. The catch-all router rule sends `none`, missing, and below-gate choices to `fast_model`; its model ID is `fast-model-id`. Supply the real model IDs and existing private-upstream opt-ins in a complete config. A `score` question returns `score`, `legend`, `probabilities`, and `confidence`. `noul` returns the probability of **yes**; low probability is not proof of safety. For `choice` routing, gate on the chosen option's probability, not its `confidence`, which describes distribution concentration. A `score` is a zero-based expected level, not a percentage.

Phase 2 wraps the typed result for PPE as:

```json
{
  "schema_version": 1,
  "status": "ok",
  "backend": "typesafe",
  "checkpoint": "jev-1.13.0",
  "question_set_id": "routing_guard_v1",
  "requested_model": "jev-1.13.0",
  "reported_model": "jev-1.13.0",
  "answers": {
    "model_tier": {
      "type": "choice",
      "choice": "deep",
      "probabilities": {"fast": 0.08, "deep": 0.89, "none": 0.03},
      "confidence": 0.71
    },
    "malicious": {"type": "noul", "noul": 0.04},
    "severity": {
      "type": "score",
      "score": 0.12,
      "legend": {"0": "None", "1": "Needs review", "2": "High risk"},
      "probabilities": {"0": 0.89, "1": 0.10, "2": 0.01},
      "confidence": 0.68
    }
  }
}
```

This is a **Praxis-owned** `custom.system_one` schema, not the external API body. Map each validated `jev::Answer` variant into this JSON explicitly: the crate's public answer types deserialize the wire response but are not a general `Serialize` interface for this PPE shape. Include `reported_model` and `usage` only when supplied. PPE rules can read paths such as `custom.system_one.answers.malicious.noul` and `custom.system_one.answers.model_tier.choice`; verify the exact APL getter syntax against the installed PPE version when implementing. The PPE rule owns thresholds and allow/review/deny. Keep `custom.llm` intact.

The PPE answer object for each configured question has exactly these decision fields:

| Question | Answer fields | Policy interpretation |
| --- | --- | --- |
| `noul` | `type: noul`, `noul` probability | Probability of the configured **yes** proposition. The policy sets its own deny/review threshold. |
| `choice` | `type: choice`, `choice`, `probabilities` keyed by every configured option, `confidence` | Publish a route only when the selected option is allowed and its own probability meets the routing gate. |
| `score` | `type: score`, numeric `score`, ordered `legend`, `probabilities`, `confidence` | `score` is a zero-based expected rubric position. The policy maps calibrated ranges to actions. |

Do not expose raw backend JSON, input state, or request body as policy output. Question IDs may appear in compatible backends' model prompts, so do not put secrets in them. The validated extension is the sole source for PPE, router publication, and branch results. A missing answer, type mismatch, missing distribution value, NaN, or out-of-range probability is an invalid backend answer, even if `jev::Evaluation` deserializes it. No consumer sees half of a multi-question evaluation.

A route-only backend failure may produce `status: unavailable`, identity fields, and an `error_kind` of `timeout`, `network`, `capacity`, `auth`, `http_status`, `oversized_response`, or `invalid_answer`, with **no** `answers`. The router must then use an explicit deterministic fallback. For `role: security`, including combined security and routing, backend failure rejects the request: 503 for timeout/network/capacity and 502 for invalid answer or unexpected status. Input errors reject regardless of role. An uncertain but valid answer is `status: ok`; a low-probability choice simply does not publish a routing header. `failure_mode: open` is not a fallback mechanism.

The route-only unavailable result has this exact stable shape; omit `answers`, `reported_model`, and `usage`:

```json
{
  "schema_version": 1,
  "status": "unavailable",
  "backend": "kev_local",
  "checkpoint": "jaredpalmer/kev-4b",
  "question_set_id": "routing_v1",
  "requested_model": "kev-latest",
  "error_kind": "timeout"
}
```

| Condition | `role: security` | `role: routing_only` |
| --- | --- | --- |
| Valid answer with publish gate met | Continue to PPE or the selected route | Continue to the selected route |
| Valid answer below publish gate | Continue to PPE; router uses its configured fallback | Router uses its configured fallback |
| Timeout, network, capacity, HTTP failure, or invalid answer | Reject; publish no answer or route header | Store `unavailable` without answers; router uses its configured fallback |
| Bad request body, unsupported media type, or input too large | Reject with the input error status | Reject with the same input error status |

The last row is never a backend fallback: the model did not receive the requested full state. For a filter that both routes and checks malicious content, choose `security`, so backend failure rejects before routing.

### Lifecycle and consumers

1. Header-only decisions use `BodyMode::Stream` and run in `on_request`. Body-derived decisions declare `BodyAccess::ReadOnly` and bounded `BodyMode::StreamBuffer`; their body hook enforces the fixed 32 KiB cap, then the decision runs from `on_request` after the complete body is available. Store the result in request extensions. Phase 2 may add `evaluate_in_pre_read` when a body-phase consumer must use the result before `on_request`.
2. Publish only validated, configured labels into reserved `x-praxis-*` headers in the hook that completes the decision. Later pre-read body hooks can see trusted pending values through `EffectiveHeaders`. Update the router to read trusted pending header mutations as well as the original header map; today a header set during `on_request` is pending and invisible to direct `ctx.request.headers` reads. Preserve existing multi-value matching for untouched headers, and reject ambiguous pending values. The router still owns cluster selection.
3. Put short configured verdicts into `FilterResultSet` in `on_request`, using the stored result, for branch conditions. Do not make a second model call or republish the header there. Keep the full answer in the typed extension. A body-dependent filter stays on the main pipeline path because branch child chains do not run body hooks.
4. Bridge the typed result into PPE's HTTP, MCP/tool, and LLM input `Extensions` as `custom.system_one`. `require_system_one: true` rejects missing or unavailable decisions.
5. For ordinary HTTP PPE with a body-derived decision, run PPE's early identity gate on the first chunk, then defer full authorization until end of body so it sees the result. For header-only decisions, authorize normally in `on_request` after the decision. Keep existing MCP/LLM end-of-body behavior.
6. For OpenAI-style routing, optionally use existing `json_body` request replacement after the decision and before PPE/router to set top-level `/model` to the selected upstream model ID. Use a fallback replacement first, then a conditional deep-model replacement based on the trusted decision header. Require `/model` to exist; `request_replace` does not create it. The route-only failure path must use the same fallback model and cluster.
7. Keep the filter before every consumer, outside branch paths that can skip a required security check. At pipeline build/reload, reject duplicate decision filters, `failure_mode: open`, skipped security paths, and `require_system_one: true` without an earlier `role: security` decision filter. Do not let model output choose an arbitrary endpoint, URL, path, header name, or cluster.

The synchronous call adds its latency to every gated request. Use Praxis's shared subrequest pool and circuit breaker. Record low-cardinality call count, duration, timeout, capacity, invalid-answer, decision-status, and reported-token metrics using configured backend/checkpoint labels. Do not label metrics by prompt, question text, arbitrary choice, tenant, request body, or backend-reported model. Do not log state or answers by default.

### Deferred use-case recipes

These recipes share one input rule: map only the required inputs into `state`. Add `headers: {from: request.headers}` when the question needs the complete post-sanitization header map. Each example needs its own calibrated question set and threshold; the numbers in the combined configuration above describe a configuration shape, not a universal threshold.

| Use case | State and fixed question | Consumer and action |
| --- | --- | --- |
| Smart model routing | Extract `/messages` from an OpenAI-style JSON body; supply static routing preferences; ask a `choice` question whose options are fixed labels such as `fast`, `deep`, `none`. | Publish only an allowed label whose own probability meets the configured gate. Router maps labels to fixed clusters and has a catch-all fallback. Optionally rewrite top-level `/model` with `json_body` before forwarding. Use `role: routing_only` only when no security decision depends on this call. |
| HTTP traffic filtering | Supply method, original path, selected query values, and the bounded request body when payload inspection is needed. Ask `noul` whether the traffic meets a specifically worded malicious condition; optionally add a severity `score` rubric. | Set `role: security`. PPE reads the structured answers and decides allow/review/deny. For body inspection, hold forwarding until the body and decision are complete. |
| LLM input guardrail | Supply text-only `/messages` and a static policy excerpt. Ask separate `noul` questions for jailbreak and sensitive-data exfiltration, or a `choice` question over fixed categories. | Set `role: security`. PPE's `cmf.llm_input` policy reads `custom.system_one` before the upstream request is sent; an unavailable decision rejects. Do not treat a low probability as proof of safety. |
| Tool-call verification | Supply the classified tool name and bounded arguments; ask a `noul` question about a specific forbidden action. | Set `role: security`. PPE's tool pre-invoke policy consumes the answer after the classifier has identified the tool. |
| Support, retrieval, and moderation | Supply bounded ticket text, query and candidate excerpt, or listing text, with fixed policy/context. Ask `choice` for a support queue, `score` for relevance, or `noul` for prohibited content. | Map configured labels to an existing router/branch, let an application read a relevance score, or let PPE review/deny a listing. Do not create a new action engine inside this filter. |

For smart routing, write down the fallback **cluster and model ID as one pair**. If the model response is unavailable or its choice is below the gate, both the router and optional `/model` rewrite must use that pair. Put the decision filter before `json_body` and router in the effective request-body/request-filter order, and ensure trusted decision headers are visible to their conditions. For any request that also uses a guardrail, the `security` role overrides route-only fallback behavior.

### Local deployment examples in Phase 2

Use the same small support-ticket state in both local examples: ticket text and product metadata, plus `queue` (`choice`: `billing`, `technical`, `other`), `urgent` (`noul`), and `severity` (`score`: `low`, `medium`, `high`). The expected answer paths are `answers.queue.choice`, `answers.urgent.noul`, and `answers.severity.score`, with distributions for `choice` and `score`. Publish only the validated `queue` label for router matching. Choose `role: security` if PPE enforces urgency or severity; use `routing_only` only if the call is solely for queue routing. Calibrate the backends separately.

**Kev, using Kev's server:**

1. Follow [Kev's local setup](https://github.com/jaredpalmer/kev#run-it-locally) for its Python/`uv` dependencies and the host's CUDA, ROCm, or Apple MLX runtime. Choose a published checkpoint such as `jaredpalmer/kev-4b`; the first launch downloads model artifacts.
2. Start `kev.serve` with `--run jaredpalmer/kev-4b --host 127.0.0.1 --port 8009`. If it uses `KEV_API_KEY`, set the corresponding Praxis `api_key_env`; omit it only for an intentionally unauthenticated local service.
3. Set Praxis `endpoint: http://127.0.0.1:8009/v1/systemone`, `allow_loopback_endpoint: true`, `backend: kev_local`, `checkpoint: jaredpalmer/kev-4b`, and wire `model: kev-latest`. A container's `127.0.0.1` is its own namespace; share a namespace or use a fixed private endpoint and `allow_private_endpoint: true`.
4. Configure the shared ticket questions and expected answer paths above. Do not use the response `model` alias as checkpoint identity.

**vLLM structured diffusion, using DiffusionGemma:**

1. Follow the [vLLM example](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/) and [deployment recipe](https://github.com/vllm-project/recipes/blob/main/models/Google/diffusiongemma-26B-A4B-it.yaml) for a supported pinned nightly build, GPU, and `google/diffusiongemma-26B-A4B-it` checkpoint.
2. Start the vLLM model server on port 8000 with `--diffusion-config '{"canvas_length":64}' --max-logprobs 32 --enable-prefix-caching`. Keep port 8000 private to the wrapper.
3. Start `examples/features/structured_diffusion/structured_server.py` with `--upstream http://127.0.0.1:8000 --tokenizer google/diffusiongemma-26B-A4B-it --canvas 64 --host 127.0.0.1 --port 8011`. Explicitly bind loopback; the wrapper's default host is broader. For a container, publish only wrapper port 8011 on host loopback.
4. Set Praxis `endpoint: http://127.0.0.1:8011/v1/systemone`, `allow_loopback_endpoint: true`, `backend: vllm_diffusion`, `checkpoint: google/diffusiongemma-26B-A4B-it`, and wire `model: jev-latest`. Praxis calls the **wrapper**, not vLLM's chat endpoint. This wire label does not select weights. If the wrapper uses `API_KEY`, set Praxis `api_key_env`.
5. Reuse the ticket questions and answer paths. Keep labels short for its tokenizer/canvas limits and set `timeout_ms` from measured local latency. The example wrapper can print answer labels; disable or redact that output for sensitive traffic. This is a separate DiffusionGemma deployment, not a Kev deployment path.

[Laya's server](https://github.com/NandhaKishorM/laya/blob/main/laya/serve.py) can use the same Phase 2 transport after its documented model names and limits are checked. The portable examples should stay within 64 questions, 2–26 choice options, 2–10 score levels, and Laya's 50,000-character state limit. A full header map may exceed a backend-specific state limit; reject it or choose a capable backend, never silently omit headers.

Create separate complete configurations, not one example that silently changes backends:

| Planned example | Contents |
| --- | --- |
| `examples/configs/security/system-one-jev.yaml` | Phase 1 hosted Jev request and request-scoped result. |
| `examples/configs/traffic_management/system-one-smart-routing.yaml` | Mapped headers/body JSON fields, `choice`, fixed cluster mapping, fallback cluster/model pair, and optional `json_body` `/model` rewrite. |
| `examples/configs/security/system-one-traffic.yaml` | Mapped method/path/headers/body state, `noul` plus severity `score`, and PPE policy. |
| `examples/configs/security/system-one-guardrails.yaml` | Text-only LLM messages, fixed safety questions, and PPE input guardrail. |
| `examples/configs/traffic_management/system-one-kev-local.yaml` | Kev's own local `/v1/systemone` server, loopback opt-in, `model: kev-latest`, actual Kev checkpoint, and routing fallback. |
| `examples/configs/traffic_management/system-one-vllm-diffusion.yaml` | vLLM DiffusionGemma plus `structured_server.py`, wrapper endpoint, `model: jev-latest`, actual DiffusionGemma checkpoint, and routing fallback. |

For each local example, document the server address/port, the exact Praxis endpoint URL, the address opt-in, the wire model label, the deployed checkpoint, and the expected answer keys. The Kev file must point to Kev's server; the vLLM file must point to its wrapper. Include `headers: {from: request.headers}` where the decision needs all headers; mappings are explicit in both examples.

### Manual implementation steps — Phase 2

1. Extend the `system_one_decision` config with the deferred fields in the table above. Validate unknown fields, enum values, endpoint permissions, question IDs and shapes, byte limits, selector names, `publish` targets, and role on startup and hot reload. Compile JSON Pointers once per built filter.
2. Extend the Phase 1 subrequest path with private/loopback endpoint opt-ins, an operator-selected whole-call deadline, state/request/response byte bounds, and a per-filter concurrency limit. Check the address actually dialed, keep HTTPS verification, deny redirects, and make no automatic retry.
3. Build `state` only from configured mappings. Add optional body JSON Pointer selectors and body-size validation; preserve the direct arbitrary-key mapping model and never add implicit headers. Keep question text and option names from validated configuration.
4. Add bounded, read-only body access for JSON selectors on the main pipeline path. Defer forwarding until end of body when the decision is required to authorize. Apply this filter's body cap even when the pipeline's shared buffer is larger; reject compressed, unsupported, or malformed input without sending a partial decision.
5. Add `evaluate_in_pre_read` only for a header-only decision needed by an earlier body-phase consumer. Document and validate the one-call ordering for header-only, pre-read, and body-derived configurations. Keep the typed result across phases in `RequestExtensions`.
6. Extend `SystemOneDecision` with role and operator-supplied backend/checkpoint/question-set identity. Map the validated `jev::Evaluation` variants explicitly into the stable `custom.system_one` JSON envelope; retain presence information for optional `model` and `usage`.
7. Implement the failure matrix above. A security or combined filter rejects on backend failure; a route-only filter publishes an `unavailable` result without answers and takes its configured fallback. Input errors always reject. Reject any configuration that would silently bypass a mandatory consumer.
8. Add `publish` projections to fixed trusted `x-praxis-*` headers and short `FilterResultSet` values. Update the existing router in `crates/filter/src/builtins/http/traffic_management/router/` to read trusted pending header mutations when matching, while preserving ordinary multi-value matching for untouched headers and rejecting ambiguous pending values. Keep the router's fixed cluster catalog and deterministic fallback.
9. Behind Praxis's `policy-engine` feature, update the `policy` filter in `crates/filter/src/builtins/http/security/policy/` to read the request-scoped result through one shared attachment helper, add `custom.system_one` to HTTP, MCP/tool, and LLM input PPE extensions, and enforce `require_system_one`. Require an earlier security-role decision filter in the pipeline. Keep existing `custom.llm` and identity handling. Verify the installed PPE getter syntax for nested answer fields.
10. For ordinary HTTP body checks, preserve PPE's early identity gate and defer final authorization until the System One body decision exists. Keep existing MCP/LLM end-of-body dispatch. Ensure no upstream body bytes leave before a required security decision.
11. For model routing, optionally configure existing `json_body` request replacement of `/model` after the decision and before forwarding. Bind the fallback model ID to the fallback cluster; configure the conditional chosen model/cluster pair using only trusted, enumerated decision labels.
12. Add the six complete example configs listed above and explain the operator setup for hosted Jev, Kev's own server, and the separate vLLM wrapper. Add a Laya compatibility example once its deployment and limits are known. Update `docs/filters/reference.md` with schema, answer paths, roles, ordering, and backend limits; include the required example header comments, then regenerate `examples/README.md` with `cargo xtask sync-example-readme --fix`.
13. Observe decisions against a held-out labeled set of normal, malicious, and out-of-distribution traffic, including prompt injection inside state. Choose each backend/checkpoint/question-set threshold using false-positive and false-negative costs, then enable enforcement or costly route selection. Record bounded duration, error-kind, and usage metrics; do not log state, secrets, or request bodies by default.

**Phase 2 exit condition:** the same request-scoped answer can drive fixed routing, optional `/model` rewriting, PPE request policy, or a branch verdict; body-derived security checks finish before forwarding; combined security/routing failures reject; route-only failures use the configured fallback; hosted Jev, local Kev, and vLLM wrapper examples follow separate documented deployment paths.

## 4. Later work and repository completion

Response/output guardrails require a separate design, especially for streaming and SSE; do not buffer an entire streaming response. Dependent questions, dynamic options, arbitrary score expressions, generic templates, and automatic provider switching wait for demonstrated needs.

This plan intentionally skips tests for now. Before merging an implementation, follow `AGENTS.md`: add unit and functional integration coverage for each new capability and example, then regenerate `examples/README.md` with `cargo xtask sync-example-readme --fix`.

Sources: [previous plan](https://gist.github.com/rikatz/ab13ed366c23ff0c7328beecfbb02b4a), [TypeSafe introduction](https://typesafe.ai/blog/introducing-system-one-models-and-jev), [System One API](https://docs.typesafe.ai/api), [primitives](https://docs.typesafe.ai/primitives), [use-case map](https://docs.typesafe.ai/concepts/use-case-map), [Rust `jev` crate API](https://docs.rs/jev/latest/jev/), [published `jev` 0.1.2 archive](https://static.crates.io/crates/jev/jev-0.1.2.crate), [Kev](https://github.com/jaredpalmer/kev), [Laya](https://github.com/NandhaKishorM/laya), [vLLM structured diffusion](https://docs.vllm.ai/en/latest/examples/features/structured_diffusion/).
