# jwe-decrypt Dynamic Module Example

This example builds the [jwe-decrypt](https://github.com/tetratelabs/built-on-envoy/tree/main/extensions/composer/jwe-decrypt)
plugin from Built On Envoy as a standalone `.so` that Praxis loads via the
Envoy dynamic module C ABI.

The plugin reads a JWE-encrypted token from a request header, decrypts it
using a configured private key, and writes the decrypted payload to an
output header.

## Prerequisites

- Go 1.26+ with CGO support
- A C toolchain (`gcc` or `clang`)
- Praxis built with `--features dynamic-modules`

## Build the Module

```console
cd examples/dynamic-modules/jwe-decrypt

# Resolve dependencies (requires network on first run).
go mod tidy

# Compile the shared library.
CGO_ENABLED=1 go build -buildmode=c-shared -o jwe-decrypt.so .
```

This produces `jwe-decrypt.so` containing:
- The `envoy_dynamic_module_on_*` event hooks (from the Go SDK's cgo exports)
- The jwe-decrypt filter logic registered under the name `"jwe-decrypt"`

## Generate a Test Key Pair

The plugin needs an RSA private key to decrypt JWE tokens.

```console
# Generate a 2048-bit RSA key pair.
openssl genpkey -algorithm RSA -out private.pem -pkeyopt rsa_keygen_bits:2048
openssl rsa -in private.pem -pubout -out public.pem
```

## Build Praxis

```console
cargo build -p praxis --features dynamic-modules
```

Verify the callback symbols are exported:

```console
nm -D target/debug/praxis | grep envoy_dynamic_module_callback | wc -l
# Should show 22+
```

## Run

```console
cargo run -p praxis --features dynamic-modules -- \
  -c examples/configs/dynamic-modules/envoy-dynamic-module.yaml
```

## Exercise

Create a JWE token using the public key (requires a JWE library or
online tool), then send it through Praxis:

```console
# With a JWE token in the Authorization header:
curl -v -H "Authorization: Bearer <JWE_TOKEN>" http://localhost:8080/

# The decrypted JWT appears in the X-Decrypted-Jwt response header
# (or whichever output_header you configured).
```

## How It Works

1. Praxis loads `jwe-decrypt.so` via `dlopen` (with `RTLD_LAZY`)
2. Calls `envoy_dynamic_module_on_program_init` — the Go SDK returns the
   ABI version
3. Calls `envoy_dynamic_module_on_http_filter_config_new` with the
   `module_name` and `module_config` from YAML — the Go SDK looks up
   `"jwe-decrypt"` in its registry and creates a config instance
4. Per request, calls `envoy_dynamic_module_on_http_filter_new` then
   `envoy_dynamic_module_on_http_filter_request_headers`
5. The plugin calls back into Praxis via `envoy_dynamic_module_callback_*`
   symbols to read/write headers

Because `RTLD_LAZY` resolves symbols on first use, only the callbacks the
plugin actually calls need to be implemented. The jwe-decrypt plugin uses:
`get_header`, `set_header`, `log`, `set_dynamic_metadata_string`, and
`get_most_specific_route_config`.
