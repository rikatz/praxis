// SPDX-License-Identifier: MIT

// Package main builds the jwe-decrypt Envoy dynamic module as a C shared
// library (.so) that Praxis can load via dlopen.
//
// The abi import pulls in cgo //export directives for all
// envoy_dynamic_module_on_* event hooks. The init function registers
// the jwe-decrypt filter factory so the SDK can find it by name when
// Praxis calls config_new with module_name "jwe-decrypt".
//
// Build:
//
//	CGO_ENABLED=1 go build -buildmode=c-shared -o jwe-decrypt.so .
package main

import "C"

import (
	// Pulls in cgo //export directives for envoy_dynamic_module_on_* symbols.
	_ "github.com/envoyproxy/envoy/source/extensions/dynamic_modules/sdk/go/abi"

	sdk "github.com/envoyproxy/envoy/source/extensions/dynamic_modules/sdk/go"
	impl "github.com/tetratelabs/built-on-envoy/extensions/composer/jwe-decrypt"
)

func init() {
	sdk.RegisterHttpFilterConfigFactories(impl.WellKnownHttpFilterConfigFactories())
}

func main() {}
