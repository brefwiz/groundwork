# Makefile for groundwork
#
# Mirrors the CI suite so contributors can reproduce CI failures locally.

.PHONY: help fmt fmt-check clippy test build clean lockfile \
        ci ci-format ci-lint ci-test ci-audit ci-coverage ci-deny ci-package

help:
	@echo "Usage: make <target>"
	@echo ""
	@echo "Development:"
	@echo "  fmt            Format code"
	@echo "  fmt-check      Check code formatting"
	@echo "  clippy         Run clippy lints"
	@echo "  test           Run tests"
	@echo "  build          Build the crate"
	@echo "  clean          Clean build artifacts"
	@echo "  lockfile       Regenerate Cargo.lock"
	@echo ""
	@echo "CI mirrors (reproduce CI failures locally):"
	@echo "  ci             Full CI suite"
	@echo "  ci-format      Check formatting"
	@echo "  ci-lint        Run clippy"
	@echo "  ci-test        Run tests via nextest"
	@echo "  ci-audit       Security audit"
	@echo "  ci-coverage    Coverage check"
	@echo "  ci-deny        Dependency license audit"
	@echo "  ci-package     Validate crate packaging"

# ============================================================================
# Development
# ============================================================================

fmt:
	cargo fmt --all

fmt-check:
	cargo fmt --all -- --check

clippy:
	cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings

test:
	cargo test --workspace

build:
	cargo build --release

clean:
	cargo clean

lockfile:
	cargo generate-lockfile

# ============================================================================
# CI suite targets (mirrors .gitea/workflows/ci.yml)
# ============================================================================

ci-format:
	cargo fmt --all -- --check

spec-check: ## L1 ADR-0086: SPEC.md exists and wire_surface is valid
	@SPEC=SPEC.md; \
	VALID="proto-source utoipa-legacy mixed-transition"; \
	[ -f "$$SPEC" ] || { echo "ERROR: $$SPEC missing (ADR-0086 L1)"; exit 1; }; \
	WS=$$(awk 'BEGIN{f=0}/^---/{f=!f;next}f&&/^wire_surface:/{print $$2;exit}' "$$SPEC"); \
	[ -n "$$WS" ] || { echo "ERROR: wire_surface field missing from $$SPEC (ADR-0086 L1)"; exit 1; }; \
	echo "$$VALID" | tr ' ' '\n' | grep -qx "$$WS" \
		|| { echo "ERROR: wire_surface='$$WS' invalid. Must be one of: $$VALID"; exit 1; }; \
	echo "spec-check OK: wire_surface=$$WS"

ci-lint: spec-check
	cargo clippy --workspace --all-targets --all-features --no-deps -- -D warnings

ci-build-check:
	cargo check --workspace --all-targets --all-features --locked

sc-001-check: ## SC-001: ban tracing_subscriber::fmt in main/bin entry points
	@grep -rn 'tracing_subscriber::fmt().*try_init\|tracing_subscriber::fmt::Subscriber.*try_init' \
	  src/main.rs src/bin/*.rs 2>/dev/null \
	  && (echo 'SC-001 violation: use otel-bootstrap via service-kit instead' && exit 1) || true

ci-test:
	cargo nextest run --all-features

ci-audit:
	cargo audit

ci-coverage:
	cargo llvm-cov nextest --all-features --lcov --output-path lcov.info \
		--fail-under-lines 80
	cargo llvm-cov report --summary-only

ci-deny:
	cargo deny check licenses

ci-package:
	cargo package --registry brefwiz --no-verify --allow-dirty

ci: ci-format ci-lint ci-test ci-audit ci-deny ci-package
	@echo "✅ All CI checks passed"

lint: ci-lint ## Alias for ci-lint
check: ci-build-check ## Alias for ci-build-check
ci-check: ci-format ci-lint ## CI: format + lint (stage 1)

ci-release-readiness: ## CI: cargo check all publishable crates
	@set -e; \
	CRATES=$$(cargo metadata --no-deps --format-version 1 2>/dev/null \
	  | python3 -c "import sys,json; [print(p['name']) for p in json.load(sys.stdin)['packages'] if p.get('publish') != []]"); \
	for pkg in $$CRATES; do echo "  checking $$pkg..."; cargo check --locked -p "$$pkg"; done

canonical-check: ## Run the brefwiz canonical structural gates locally (ADR-0086)
	@if [ -x .ci-workflows/ci-scripts/canonical-check.sh ]; then \
		bash .ci-workflows/ci-scripts/canonical-check.sh; \
	else echo "canonical-check: .ci-workflows script not present locally — runs in CI"; fi

cds-lint: ## Validate .cds/workflows/ YAML via cdsctl
	@if [ -x /tmp/ci-workflows/ci-scripts/cds-lint.sh ]; then \
		bash /tmp/ci-workflows/ci-scripts/cds-lint.sh; \
	else echo "cds-lint: ci-workflows script not present locally — runs in CI"; fi

ci-fmt: ci-format ## CI: format check (alias)

ci-lockfile-diff: ## CI: assert committed Cargo.lock matches Linux resolution (no-op — lock regenerated on Linux in CI)
	@true

.PHONY: pre-commit
pre-commit: ci-format ci-lint ci-test ci-changelog ## Run all pre-commit checks (ADR-0021)

.PHONY: ci-changelog
ci-changelog: ## CI: verify CHANGELOG.md has entry for current package version (ADR-0021)
	@if [ -x .ci-workflows/ci-scripts/check-release-changelog.sh ]; then \
		bash .ci-workflows/ci-scripts/check-release-changelog.sh; \
	elif command -v check-release-changelog.sh >/dev/null 2>&1; then \
		check-release-changelog.sh; \
	else \
		echo "ci-changelog: check-release-changelog.sh not found; skipping (run via CI for the full check)"; \
	fi
