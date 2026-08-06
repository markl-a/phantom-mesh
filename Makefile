# spectyn-mesh — quickstart targets
#
# Match the quickstart UX of the satellite repos (spectyn-secops, spectyn-mobile):
# `make demo` should produce visible output in <60s on a developer laptop.
#
# Targets:
#   make help              list all targets
#   make install           cargo install --path core (puts `spectyn` on PATH)
#   make doctor            spectyn doctor (9-section health check)
#   make test              cargo test (workspace-wide)
#   make build             cargo build --release
#   make demo              spectyn serve + a sample query against the local agent
#   make ecosystem-demo    runs spectyn-secops + spectyn-mobile mock demos
#                          (requires the satellite repos cloned as siblings)
#   make clean             cargo clean

.PHONY: help install doctor test build demo ecosystem-demo clean check selftest selftest-json selftest-list ios-rebuild

ROOT := $(shell pwd)
SATELLITES := $(ROOT)/../spectyn-secops $(ROOT)/../spectyn-mobile

help:  ## Show this help
	@awk 'BEGIN{FS=":.*##"} /^[a-zA-Z_-]+:.*##/ {printf "  %-18s %s\n", $$1, $$2}' $(MAKEFILE_LIST)

install:  ## cargo install spectyn (places binary on PATH)
	cd core && cargo install --path . --locked

doctor:  ## Run spectyn's 9-section health check
	@command -v spectyn >/dev/null 2>&1 || { echo "spectyn not on PATH — run 'make install' first"; exit 1; }
	spectyn doctor

selftest:  ## Run the cross-feature self-test suite (text output)
	@bash scripts/selftest.sh

selftest-json:  ## Run the self-test suite and write JSON to test-results/
	@mkdir -p test-results
	@bash scripts/selftest.sh --json --out test-results/selftest-$$(date +%Y%m%d-%H%M%S).json
	@echo "→ latest reports:" && ls -t test-results/selftest-*.json 2>/dev/null | head -3

selftest-list:  ## List registered self-test features
	@bash scripts/selftest.sh --list

check:  ## cargo check (compiles without producing the final binary)
	cd core && cargo check

test:  ## cargo test (workspace-wide)
	cd core && cargo test

build:  ## cargo build --release
	cd core && cargo build --release --bin spectyn

demo:  ## Start spectyn serve in the background + run a sample query
	@command -v spectyn >/dev/null 2>&1 || { echo "spectyn not on PATH — run 'make install' first"; exit 1; }
	@echo "→ checking spectyn serve health..."
	@if curl -sf http://127.0.0.1:7878/healthz >/dev/null 2>&1; then \
		echo "  ✓ spectyn serve already running at :7878"; \
	else \
		echo "  ! spectyn serve not running — start with 'spectyn serve' in another terminal"; \
		exit 1; \
	fi
	@echo "→ asking spectyn a sample question..."
	spectyn run "show the file count of this repo using shell tools"

ecosystem-demo:  ## Run the satellite mock demos (secops + mobile) in sequence
	@for s in $(SATELLITES); do \
		if [ -d "$$s" ]; then \
			echo ""; \
			echo "═══════════════════════════════════════════════════════════"; \
			echo "  $$(basename $$s) demo"; \
			echo "═══════════════════════════════════════════════════════════"; \
			$(MAKE) -C $$s demo-mock || exit 1; \
		else \
			echo "  (skip: $$s not cloned as sibling)"; \
		fi; \
	done

clean:  ## cargo clean
	cd core && cargo clean

ios-rebuild:  ## Rebuild + re-sign iOS IPA (use weekly to refresh 7-day free-cert expiry)
	@if [ -z "$$APPLE_TEAM_ID$$DEVELOPMENT_TEAM" ]; then \
		echo "❌  Set APPLE_TEAM_ID before running (10-char team id from your dev cert)."; \
		echo "    Find yours: security find-identity -v -p codesigning | grep Apple"; \
		exit 1; \
	fi
	@bash scripts/package-ios.sh
	@if [ -f dist/spectyn-mesh-ios.ipa ]; then \
		echo ""; \
		echo "✓ Fresh IPA at dist/spectyn-mesh-ios.ipa — sideload within 7 days."; \
	fi
