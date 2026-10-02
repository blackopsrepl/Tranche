# Tranche Makefile
# Jev-powered triage pipeline with colorized output

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
# One interpreter for every target. `PYTHON` (env or command line) wins;
# otherwise hostpython.py resolves it.
PYTHON_CMD ?= python3
ifeq ($(strip $(PYTHON)),)
PYTHON := $(shell $(PYTHON_CMD) hostpython.py 2>/dev/null)
endif
ifeq ($(strip $(PYTHON)),)
$(error No usable Python interpreter: '$(PYTHON_CMD) hostpython.py' failed)
endif
JUDGED := $(shell test -f out/judgments.jsonl && wc -l < out/judgments.jsonl || echo 0)
PAIRED := $(shell test -f out/pair_verdicts.jsonl && wc -l < out/pair_verdicts.jsonl || echo 0)

# ============== Phony Targets ==============
.PHONY: banner help fetch refresh judge judge-full dupes cluster page gif all publish verify info clean-judgments test check mcp-check print-interpreter release-check release-dry-run release

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
	@$(PYTHON) tranche.py fetch --transport gh

# ============== Jev Pipeline ==============

judge: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║        Jev Judgment Pass             ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@printf "$(ARROW) $(BOLD)Judging PRs (7 typed questions, one batched call each)...$(RESET)\n"
	@$(PYTHON) tranche.py judge --resume && \
		printf "$(GREEN)$(CHECK) Judgments saved to out/judgments.jsonl$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Judge pass failed$(RESET)\n\n" && exit 1)

judge-full: banner
	@printf "$(RED)$(BOLD)WARNING: fresh pass over all PRs — ~5M input tokens on Jev$(RESET)\n"
	@printf "$(YELLOW)Press Ctrl+C to abort, or Enter to continue...$(RESET)\n"
	@read dummy
	@$(PYTHON) tranche.py judge

dupes: banner
	@printf "$(ARROW) $(BOLD)Comparing candidate pairs with Jev sameness judgments...$(RESET)\n"
	@$(PYTHON) tranche.py dupes && \
		printf "$(GREEN)$(CHECK) Pair verdicts in out/pair_verdicts.jsonl$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Dupe pass failed$(RESET)\n\n" && exit 1)

cluster: banner
	@printf "$(ARROW) $(BOLD)Clustering tranches, dupes, escalation lists...$(RESET)\n"
	@$(PYTHON) tranche.py cluster

# ============== Output ==============

page: banner
	@printf "$(ARROW) $(BOLD)Rendering GitHub Pages report from out/ data...$(RESET)\n"
	@$(PYTHON) gen_page.py && \
		printf "$(GREEN)$(CHECK) docs/index.html written$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) Page generation failed$(RESET)\n\n" && exit 1)

gif: banner
	@printf "$(ARROW) $(BOLD)Rendering title GIF with glyphfx (capture → rasterize)...$(RESET)\n"
	@$(PYTHON) tools/make_title_gif.py && \
		printf "$(GREEN)$(CHECK) docs/assets/tranche.gif written$(RESET)\n\n" || \
		(printf "$(RED)$(CROSS) GIF generation failed$(RESET)\n\n" && exit 1)

# ============== Composite Targets ==============

refresh: banner
	@printf "$(CYAN)$(BOLD)╔══════════════════════════════════════╗$(RESET)\n"
	@printf "$(CYAN)$(BOLD)║     Incremental Refresh (all)        ║$(RESET)\n"
	@printf "$(CYAN)$(BOLD)╚══════════════════════════════════════╝$(RESET)\n\n"
	@$(PYTHON) tranche.py refresh --max-pairs $(if $(MAX_PAIRS),$(MAX_PAIRS),400)

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
	@$(PYTHON) -c "import json; s=json.load(open('out/summary.json')); [print(f'  $(CYAN){k:>22}$(RESET)  {v}') for k,v in s.items()]"
	@printf "\n"

# ============== Danger Zone ==============

clean-judgments: banner
	@printf "$(RED)$(BOLD)WARNING: deletes all Jev judgments (~5M tokens to redo)$(RESET)\n"
	@printf "$(YELLOW)Press Ctrl+C to abort, or Enter to continue...$(RESET)\n"
	@read dummy
	@rm -fv out/judgments.jsonl out/pair_verdicts.jsonl && \
		printf "$(GREEN)$(CHECK) Judgment cache cleared$(RESET)\n\n"

test:
	@$(PYTHON) -m unittest discover -s tests -v

check: test
	@if command -v node >/dev/null 2>&1; then node --test tests/workbench.test.cjs; else printf 'Node unavailable; optional frontend tests skipped.\n'; fi
	@ruff check .
	@$(PYTHON) -m compileall -q tranche.py gen_page.py hostpython.py mcp_server.py tests tools
	@git diff --check

# Real-client subprocess check against the current local corpus, separate from
# the offline gates. MCP_PYTHON supplies the MCP client SDK; the server imports
# nothing, so it defaults to the same interpreter.
MCP_PYTHON ?= $(PYTHON)
mcp-check:
	@TRANCHE_MCP_INTEGRATION=1 $(MCP_PYTHON) -m unittest tests.test_mcp_stdio -v

print-interpreter:
	@printf '%s\n' '$(PYTHON)'

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
	@/bin/echo -e "  $(GREEN)make refresh$(RESET)       - $(BOLD)Deterministic incremental refresh of everything$(RESET)"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Output:$(RESET)"
	@/bin/echo -e "  $(GREEN)make page$(RESET)          - Render docs/index.html from out/ data"
	@/bin/echo -e "  $(GREEN)make gif$(RESET)           - Re-render the glyphfx title GIF"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Composite:$(RESET)"
	@/bin/echo -e "  $(GREEN)make all$(RESET)           - $(YELLOW)$(BOLD)fetch → judge → dupes → cluster → page$(RESET)"
	@/bin/echo -e "  $(GREEN)make publish$(RESET)       - Commit docs/ + push (Pages rebuilds)"
	@/bin/echo -e "  $(GREEN)make verify$(RESET)        - Prove the live page serves"
	@/bin/echo -e ""
	@/bin/echo -e "$(CYAN)$(BOLD)Release:$(RESET)"
	@/bin/echo -e "  $(GREEN)make check$(RESET)         - Offline tests, Ruff, syntax and whitespace"
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
