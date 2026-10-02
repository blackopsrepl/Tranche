# Tranche Makefile
# Jev-powered triage pipeline, native Rust

# ============== Colors & Symbols ==============
GREEN := \033[92m
EMERALD := \033[38;2;16;185;129m
CYAN := \033[96m
YELLOW := \033[93m
MAGENTA := \033[95m
RED := \033[91m
GRAY := \033[90m
BOLD := \033[1m
RESET := \033[0m

CHECK := ✓
CROSS := ✗
ARROW := ▸
PROGRESS := →
PEOPLE := 👥
SCALE := ⚖️

# ============== Project Metadata ==============
REPO := blackopsrepl/Tranche
LIVE_URL := https://vdistefano.studio/Tranche/
JUDGED := $(shell test -f out/judgments.jsonl && wc -l < out/judgments.jsonl || echo 0)
PAIRED := $(shell test -f out/pair_verdicts.jsonl && wc -l < out/pair_verdicts.jsonl || echo 0)
# The pipeline is the native binary. `tranche` resolves from PATH;
# `make cli-install` builds and installs it from this checkout. Override with
# TRANCHE=./target/debug/tranche to run an uninstalled build.
TRANCHE ?= tranche

# ============== Phony Targets ==============
.PHONY: banner help fetch refresh judge judge-full dupes cluster batches page all evidence evidence-show publish verify info clean-judgments test check cli-install release-check release-dry-run release

# ============== Default Target ==============
.DEFAULT_GOAL := help

# ============== Banner ==============
banner:
	@printf "$(EMERALD)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(EMERALD)$(BOLD)║               TRANCHE                ║$(RESET)\n"
	@printf "$(EMERALD)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n"
	@printf "$(GRAY)  Model-assisted PR review · x Jev$(RESET)\n"
	@printf "  $(GRAY)judged: $(CYAN)$(JUDGED)$(GRAY) PRs, $(CYAN)$(PAIRED)$(GRAY) pair verdicts$(RESET)\n"
	@printf "  $(GRAY)$(LIVE_URL)$(RESET)\n\n"

# ============== Data ==============

fetch: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║        Fetching Open PRs             ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@$(TRANCHE) fetch --transport gh

# ============== Jev Pipeline ==============

judge: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║        Jev Judgment Pass             ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@printf "$(ARROW) $(BOLD)Judging PRs (7 typed questions, one batched call each)...$(RESET)\n"
	@$(TRANCHE) judge --resume && \
		printf "$(GREEN)$(CHECK) Judgments saved to out/judgments.jsonl$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Judge pass failed$(RESET)\n\n" && exit 1)

judge-full: banner
	@printf "$(RED)$(BOLD)WARNING: fresh pass over all PRs — ~5M input tokens on Jev$(RESET)\n"
	@printf "$(YELLOW)Press Ctrl+C to abort, or Enter to continue...$(RESET)\n"
	@read dummy
	@$(TRANCHE) judge

dupes: banner
	@printf "$(ARROW) $(BOLD)Comparing candidate pairs with Jev sameness judgments...$(RESET)\n"
	@$(TRANCHE) dupes && \
		printf "$(GREEN)$(CHECK) Pair verdicts in out/pair_verdicts.jsonl$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Dupe pass failed$(RESET)\n\n" && exit 1)

cluster: banner
	@printf "$(ARROW) $(BOLD)Clustering tranches, dupes, escalation lists...$(RESET)\n"
	@$(TRANCHE) cluster

# Issue #4/#8: cumulative pre-release batches and the park record.
batches: banner
	@printf "$(ARROW) $(BOLD)Classifying review candidates into cumulative batches...$(RESET)\n"
	@$(TRANCHE) batches

# ============== Output ==============

page: banner
	@printf "$(ARROW) $(BOLD)Rendering GitHub Pages report from out/ data...$(RESET)\n"
	@$(TRANCHE) page && \
		printf "$(GREEN)$(CHECK) docs/index.html written$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Page generation failed$(RESET)\n\n" && exit 1)

# ============== Composite Targets ==============

refresh: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║     Incremental Refresh (all)        ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@$(TRANCHE) refresh --max-pairs $(if $(MAX_PAIRS),$(MAX_PAIRS),400)

# Issue #9: capture the public evidence of one native batch, locally and ignored.
evidence: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║     Evidence capture (batch)          ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@test -n "$(BATCH)" || (printf "$(RED)Usage: make evidence BATCH=B001 [BUDGET=200]$(RESET)\n" && exit 2)
	@$(TRANCHE) evidence capture --batch $(BATCH) --request-budget $(if $(BUDGET),$(BUDGET),200)

evidence-show: banner
	@$(TRANCHE) evidence show --batch $(BATCH)

all:
	@$(MAKE) fetch
	@$(MAKE) judge
	@$(MAKE) dupes
	@$(MAKE) cluster
	@$(MAKE) page
	@printf "$(GREEN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(GREEN)$(BOLD)║        $(CHECK) PIPELINE COMPLETE              ║$(RESET)\n"
	@printf "$(GREEN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n"
	@printf "$(GRAY)Run 'make publish' to ship it.$(RESET)\n\n"

publish: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║        Publishing to Pages           ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@git add -A docs/ && \
		git diff --cached --quiet && \
			printf "$(YELLOW)Nothing new to publish$(RESET)\n\n" || \
		( git commit -qm "chore: refresh triage report" && git push -q origin master && \
		  printf "$(GREEN)$(CHECK) Pushed — Pages rebuilds at $(CYAN)$(LIVE_URL)$(RESET)\n" && \
		  printf "$(GRAY)Watch: gh api repos/$(REPO)/pages --jq .status$(RESET)\n\n" )

verify: banner
	@printf "$(ARROW) $(BOLD)Proving the release is live...$(RESET)\n"
	@gh api repos/$(REPO)/pages --jq '"  status: " + .status + "  (https enforced: " + (.https_enforced|tostring) + ")"'
	@code=$$(curl -s -o /tmp/verify.html -w "%{http_code}" -L "$(LIVE_URL)") ; \
		[ "$$code" = "200" ] && grep -q "TRANCHE" /tmp/verify.html && \
		printf "$(GREEN)$(CHECK) 200 + content at $(LIVE_URL)$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Live check failed (HTTP $$code)$(RESET)\n\n" && exit 1)

info: banner
	@$(TRANCHE) info

# ============== Danger Zone ==============

clean-judgments: banner
	@printf "$(RED)$(BOLD)WARNING: deletes all Jev judgments (~5M tokens to redo)$(RESET)\n"
	@printf "$(YELLOW)Press Ctrl+C to abort, or Enter to continue...$(RESET)\n"
	@read dummy
	@rm -fv out/judgments.jsonl out/pair_verdicts.jsonl && \
		printf "$(GREEN)$(CHECK) Judgment cache cleared$(RESET)\n\n"

test:
	@cargo test --locked

check: test
	@cargo fmt --check
	@cargo clippy --locked --all-targets -- -D warnings
	@if command -v node >/dev/null 2>&1; then node --test tests/*.test.cjs; else printf 'Node unavailable; optional frontend and release tests skipped.\n'; fi
	@git diff --check

cli-install:
	@cargo install --path crates/tranche-cli --locked

release-check: check
	@node --check .versionrc.js
	@test "$$(git branch --show-current)" = master || (printf 'Release from master only.\n' >&2; exit 1)
	@test -z "$$(git status --porcelain)" || (printf 'Commit or remove working-tree changes before releasing.\n' >&2; exit 1)

release-dry-run: release-check
	@commit-and-tag-version --dry-run

release:
	@commit-and-tag-version

# ============== Help ==============

help: banner
	@/bin/echo -e "$(CYAN)$(BOLD)Pipeline:$(RESET)"
	@/bin/echo -e "  $(GREEN)make fetch$(RESET)         - Refresh open-PR snapshot (authenticated via gh)"
	@/bin/echo -e "  $(GREEN)make judge$(RESET)         - Jev pass over unjudged PRs (resume-safe)"
	@/bin/echo -e "  $(GREEN)make dupes$(RESET)         - Compare candidate pairs with Jev"
	@/bin/echo -e "  $(GREEN)make cluster$(RESET)       - Build tranches, dupe groups, escalation lists"
	@/bin/echo -e "  $(GREEN)make batches$(RESET)       - Build cumulative pre-release batches and the park record"
	@/bin/echo -e "  $(GREEN)make refresh$(RESET)       - $(BOLD)Deterministic incremental refresh of everything$(RESET)"
	@/bin/echo -e "  $(GRAY)Every stage runs the native frontend: tranche fetch|judge|dupes|cluster|batches|refresh|page$(RESET)"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Output:$(RESET)"
	@/bin/echo -e "  $(GREEN)make page$(RESET)          - Render docs/index.html from out/ data"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Composite:$(RESET)"
	@/bin/echo -e "  $(GREEN)make all$(RESET)           - $(YELLOW)$(BOLD)fetch → judge → dupes → cluster → page$(RESET)"
	@/bin/echo -e "  $(GREEN)make publish$(RESET)       - Commit docs/ + push (Pages rebuilds)"
	@/bin/echo -e "  $(GREEN)make verify$(RESET)        - Prove the live page serves"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Evidence (issue #9):$(RESET)"
	@/bin/echo -e "  $(GREEN)make evidence BATCH=B001$(RESET) - Capture one batch's public evidence (read-only)"
	@/bin/echo -e "  $(GREEN)make evidence-show BATCH=B001$(RESET) - Inspect the current batch's coverage"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Rust toolchain:$(RESET)"
	@/bin/echo -e "  $(GREEN)make cli-install$(RESET)   - Build and install the tranche binary from this checkout"
	@/bin/echo -e "  $(GRAY)Override the binary with TRANCHE=./target/debug/tranche$(RESET)"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Release:$(RESET)"
	@/bin/echo -e "  $(GREEN)make test$(RESET)          - Cargo tests over the workspace"
	@/bin/echo -e "  $(GREEN)make check$(RESET)         - Tests plus format, clippy and whitespace"
	@/bin/echo -e "  $(GREEN)make release-dry-run$(RESET) - Preview version/changelog on clean master"
	@/bin/echo -e "  $(GREEN)make release$(RESET)       - Gate, bump VERSION, generate changelog, commit + tag"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Other:$(RESET)"
	@/bin/echo -e "  $(GREEN)make info$(RESET)          - Show summary.json numbers"
	@/bin/echo -e "  $(GREEN)make judge-full$(RESET)    - $(RED)Fresh judgment pass (burns ~5M tokens)$(RESET)"
	@/bin/echo -e "  $(GREEN)make clean-judgments$(RESET) - $(RED)Delete the judgment cache$(RESET)"
	@/bin/echo -e "  $(GREEN)make help$(RESET)          - Show this help message"
	@/bin/echo -e ""
	@/bin/echo -e "$(GRAY)API key: ~/Documents/jevapi.txt  ·  Model: jev-latest (alias; resolved version recorded when returned)$(RESET)"
	@/bin/echo -e ""
