## register-rs — task entry points. Routes through Docker unless INSIDE_CONTAINER=1.

inside := env_var_or_default("INSIDE_CONTAINER", "0")

dev_running := `docker compose ps --status running --services 2>/dev/null | grep -c '^dev$' 2>/dev/null || true`
docker_run := if dev_running == "0" { "docker compose run --rm dev" } else { "docker compose exec dev" }

cargo := if inside == "1" { "cargo" } else { docker_run + " cargo" }
rustup := if inside == "1" { "rustup" } else { docker_run + " rustup" }
typos := if inside == "1" { "typos" } else { docker_run + " typos" }
actionlint := if inside == "1" { "actionlint" } else { docker_run + " actionlint" }
lefthook := if inside == "1" { "lefthook" } else { docker_run + " lefthook" }
taplo := if inside == "1" { "taplo" } else { docker_run + " taplo" }
biome := if inside == "1" { "biome" } else { docker_run + " biome" }
yamlfmt := if inside == "1" { "yamlfmt" } else { docker_run + " yamlfmt" }
img2pdf := if inside == "1" { "img2pdf" } else { docker_run + " img2pdf" }
sh := if inside == "1" { "bash -lc" } else { docker_run + " bash -lc" }

dev_log := env_var_or_default("REGISTER_LOG", "info")

default:
    @just --list

# ----- first-run bootstrap -----

bootstrap:
    @echo "==> 1/3 fetch dev image (try ghcr.io, fall back to local build)"
    @docker compose pull 2>/dev/null && echo "  (pulled prebuilt image from ghcr.io)" \
        || (echo "  (no published image, building locally with GITHUB_TOKEN if available)" && \
            GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" docker compose build)
    @echo "==> 2/3 docker compose up -d dev (persistent dev container)"
    docker compose up -d dev
    @echo "==> 3/3 lefthook install (pre-commit / pre-push hooks)"
    {{lefthook}} install
    @just doctor
    @echo
    @echo "🎉 bootstrap done. Try: just build / just test / just lint"

doctor:
    @echo "==> register-rs doctor"
    @{{docker_run}} bash -c 'set -e; \
        check() { printf "  %-18s " "$1"; out=$($2 2>&1 | head -1) && printf "ok    %s\n" "$out" || { printf "MISSING\n"; exit 1; }; }; \
        check rustc          "rustc --version"; \
        check cargo          "cargo --version"; \
        check cargo-nextest  "cargo nextest --version"; \
        check cargo-deny     "cargo deny --version"; \
        check cargo-audit    "cargo audit --version"; \
        check cargo-llvm-cov "cargo llvm-cov --version"; \
        check cargo-machete  "cargo machete --version"; \
        check cargo-sort     "cargo sort --version"; \
        check cargo-rdme     "cargo rdme --version"; \
        check cargo-modules  "cargo modules --version"; \
        check cargo-depgraph "cargo depgraph --version"; \
        check typos          "typos --version"; \
        check taplo          "taplo --version"; \
        check biome          "biome --version"; \
        check yamlfmt        "yamlfmt --version"; \
        check actionlint     "actionlint -version"; \
        check lefthook       "lefthook version"; \
        check just           "just --version"; \
        check mold           "mold --version"; \
        check clang          "clang --version"; \
    '
    @echo "==> doctor: ok"

# ----- one-shot environment -----

docker-build:
    @echo "==> docker compose build (GITHUB_TOKEN auto-loaded from gh CLI if available)"
    GITHUB_TOKEN="${GITHUB_TOKEN:-$(gh auth token 2>/dev/null || true)}" docker compose build

shell:
    {{docker_run}} bash

clean-docker:
    @echo "==> docker compose down (volumes + local images)"
    docker compose down --volumes --rmi local

dev-up:
    @echo "==> docker compose up -d dev"
    docker compose up -d dev
    @echo "dev container is up — `just <recipe>` now uses docker exec (faster)."

dev-down:
    docker compose stop dev

# ----- Rust workflow -----

build:
    @echo "==> cargo build --workspace --all-targets"
    {{cargo}} build --workspace --all-targets

build-release:
    {{cargo}} build --release --workspace

b:
    {{cargo}} build --workspace

test:
    @echo "==> cargo nextest run --workspace"
    {{cargo}} nextest run --workspace
    @echo "==> cargo test --doc --workspace"
    {{cargo}} test --doc --workspace

t:
    {{cargo}} nextest run --workspace --no-fail-fast

doctest:
    {{cargo}} test --doc --workspace

coverage:
    {{cargo}} llvm-cov --workspace --branch --html --output-dir artifacts/coverage

# ----- run -----

# Register a directory of bitonal images onto the target paper canvas.
run input output *args:
    REGISTER_LOG={{dev_log}} {{cargo}} run -p register-cli -- {{input}} {{output}} {{args}}

run-release input output *args:
    REGISTER_LOG={{dev_log}} {{cargo}} run --release -p register-cli -- {{input}} {{output}} {{args}}

# Process the bundled `samples/` directory into `artifacts/sample-out`.
# Used as a smoke check during development.
run-sample:
    @mkdir -p artifacts
    REGISTER_LOG={{dev_log}} {{cargo}} run -p register-cli -- \
        samples artifacts/sample-out --force

# Roll a directory of PBM/PNG pages into a single PDF for human review.
# Looking at a few hundred bitonal pages one-by-one is unworkable; pack
# them so you can scrub through in a normal PDF viewer.
#
# Example: `just to-pdf artifacts/russell-out artifacts/russell.pdf`
to-pdf in out:
    {{sh}} 'mkdir -p "$(dirname "{{out}}")" && {{img2pdf}} {{in}}/* --output {{out}}'

# Bulk: pack every register output directory under `artifacts/*-out/` into
# `artifacts/*-registered.pdf`. Useful after a batch run; gives one PDF per
# book that you can flip through to spot-check alignment.
to-all-pdfs:
    {{docker_run}} bash -c '\
        set -euo pipefail; \
        for dir in artifacts/*-out; do \
            [ -d "$dir" ] || continue; \
            book="$(basename "$dir" -out)"; \
            out="artifacts/${book}-registered.pdf"; \
            echo "==> $dir -> $out"; \
            img2pdf "$dir"/* --output "$out"; \
        done'

# Pack BOTH the input directory and the register output into separate PDFs
# (`*-original.pdf` and `*-registered.pdf`). Open them in two side-by-side
# viewers to eyeball what register actually moved — the input page edges
# wobble, the output edges don't.
#
# Example: `just diff-pdf private/extracted/russell artifacts/russell-out artifacts/russell`
diff-pdf in_orig in_registered out_prefix:
    {{sh}} 'mkdir -p "$(dirname "{{out_prefix}}")" && \
        {{img2pdf}} {{in_orig}}/* --output {{out_prefix}}-original.pdf && \
        {{img2pdf}} {{in_registered}}/* --output {{out_prefix}}-registered.pdf'

# ----- Python visualizers (host: uv tools/, container: baked in) -----
#
# All three render a different "did register align anything?" view from a
# `(before-dir, after-dir)` pair. Heavy lifting in numpy; img2pdf wraps the
# per-page ONGs into a single scrubbable PDF.

py := if inside == "1" { "python" } else { docker_run + " python" }
uv := if inside == "1" { "uv" } else { docker_run + " uv" }
ruff := if inside == "1" { "ruff" } else { docker_run + " ruff" }
mypy := if inside == "1" { "mypy" } else { docker_run + " uv tool run mypy" }

# Per-page color-coded overlay PDF: red = ink in original only, green = ink
# in registered only, black = unchanged, white = paper. Shows EVERYTHING
# (text + delta) — useful when you also want to see the page content.
overlay-diff before after out_pdf:
    {{uv}} run --with pillow --with numpy python tools/overlay_diff.py \
        {{before}} {{after}} {{out_pdf}}

# Per-page delta-only PDF: red+green only, no static text. White everywhere
# nothing changed, so the diff is *just* the alignment shift. The cleanest
# view for "show me what register did".
delta-diff before after out_pdf:
    {{uv}} run --with pillow --with numpy python tools/diff_pages.py \
        --before {{before}} --after {{after}} --output {{out_pdf}}

# Single-image proof: every page's ink bounding box drawn on one shared
# canvas. Red = bboxes from the unprocessed corpus (centered on canvas),
# green = bboxes from the registered corpus. Tight green cluster + loose
# red cloud means alignment worked.
bbox-overlay before after out_png:
    {{uv}} run --with pillow --with numpy python tools/bbox_overlay.py \
        --before {{before}} --after {{after}} --output {{out_png}}

# Corpus-wide stacked composite: per-pixel ink density over every page,
# rendered side-by-side (before | after). Sharper text on the right means
# pages line up. Slow on 300+ page corpora.
stack-corpus before after out_png:
    {{uv}} run --with pillow --with numpy python tools/corpus_stack.py \
        --before {{before}} --after {{after}} --output {{out_png}}

# ----- Python lint / format / typecheck (tools/) -----

py-lint:
    {{ruff}} check tools

py-fmt:
    {{ruff}} check --fix tools
    {{ruff}} format tools

py-fmt-check:
    {{ruff}} format --check tools

py-typecheck:
    {{mypy}} tools

# ----- lint / quality gates -----

fmt:
    {{cargo}} fmt --all
    {{cargo}} sort --workspace
    {{taplo}} fmt
    {{biome}} format --write .
    {{yamlfmt}} .

fmt-check:
    {{cargo}} fmt --all -- --check
    {{cargo}} sort --workspace --check
    {{taplo}} fmt --check
    {{biome}} format .
    {{yamlfmt}} --lint .

clippy:
    {{cargo}} clippy --workspace --all-targets -- -D warnings

deny:
    {{cargo}} deny check advisories bans licenses sources

audit:
    {{cargo}} audit --deny warnings

typos:
    {{typos}}

typos-fix:
    {{typos}} --write-changes

actionlint:
    {{actionlint}} .github/workflows/*.yml

machete:
    {{cargo}} machete

# ----- auto-generated docs -----

docs-dep-graph:
    {{sh}} "{{cargo}} depgraph --workspace-only | dot -Tsvg > docs/dep-graph.svg"

docs-modules:
    {{sh}} "{{cargo}} modules structure --package register-core > docs/modules/register-core.txt"
    {{sh}} "{{cargo}} modules structure --package register-cli > docs/modules/register-cli.txt"

docs-readme:
    {{sh}} "cd crates/register-core && {{cargo}} rdme --force"

docs: docs-dep-graph docs-modules docs-readme

doc:
    {{cargo}} doc --workspace --no-deps --open

# ----- profiling -----

# Profile a corpus run with samply and open the Firefox Profiler UI on the
# result. Uses the `profiling` cargo profile so source-line / frame-pointer
# information survives full release optimization.
#
# Usage:
#   just profile-run private/extracted/morimasato            # default DPI 300
#   just profile-run private/extracted/syousatsu --dpi 200   # extra flags pass through
profile-run input *args:
    @mkdir -p artifacts/profiles
    {{cargo}} build --profile profiling -p register-cli
    @echo "==> samply record (paranoid={{ `cat /proc/sys/kernel/perf_event_paranoid 2>/dev/null || echo unknown` }}, output: artifacts/profiles/latest.json)"
    samply record --output artifacts/profiles/latest.json \
        ./target/profiling/register {{input}} artifacts/profile-output --force --paper b5 {{args}}

# Same as `profile-run` but skips opening the UI — useful when you just want
# to save profiles for diffing later. Resulting JSON loads on
# https://profiler.firefox.com.
profile-save input *args:
    @mkdir -p artifacts/profiles
    {{cargo}} build --profile profiling -p register-cli
    @ts=$(date -u +%Y%m%dT%H%M%SZ); out=artifacts/profiles/profile-$ts.json; \
    echo "==> samply record → $out"; \
    samply record --save-only --output $out \
        ./target/profiling/register {{input}} artifacts/profile-output --force --paper b5 {{args}} ; \
    echo "==> wrote $out (load on https://profiler.firefox.com)"

# Run the criterion bench suite and dump a Markdown summary of latest numbers
# next to the HTML reports. The HTML lands in target/criterion.
bench:
    {{cargo}} bench --bench pipeline
    @echo "==> criterion HTML: target/criterion/report/index.html"

# Quick bench — `--quick` mode for fast feedback during iteration.
bench-quick filter="":
    {{cargo}} bench --bench pipeline -- {{filter}} --quick

# Take a named criterion baseline. Subsequent `cargo bench` runs print
# `change: ±x.x%` against this baseline until you save a new one. Pair
# with `bench` to capture before/after on architectural changes.
#
#   just bench-baseline-save before-rayon-canvas-pool
#   # …make change…
#   just bench-baseline-cmp before-rayon-canvas-pool
bench-baseline-save name:
    {{cargo}} bench --bench pipeline -- --save-baseline {{name}}

bench-baseline-cmp name:
    {{cargo}} bench --bench pipeline -- --baseline {{name}}

# Time the end-to-end CLI N times against a real corpus, print median +
# spread. Bypasses criterion (which only measures library code) to capture
# I/O time too. Default corpus is the 343-page private/extracted/morimasato
# (held out of the repo). Override `corpus=` to point elsewhere.
e2e-bench corpus="private/extracted/morimasato" n="7":
    @just _e2e-bench-impl {{corpus}} {{n}}

_e2e-bench-impl corpus n:
    @{{cargo}} build --release -p register-cli >/dev/null
    @echo "==> end-to-end timing: {{corpus}} ({{n}} runs, wall seconds)"
    @samples=$$(for i in $$(seq 1 {{n}}); do /usr/bin/time -f "%e" ./target/release/register {{corpus}} artifacts/e2e-bench-out --paper b5 --dpi 300 --force 2>&1 | tail -1; done); \
    echo "$$samples" | awk 'BEGIN { min=999; max=0 } { s+=$$1; n+=1; if($$1<min) min=$$1; if($$1>max) max=$$1; v[n]=$$1 } END { asort(v); print "  median:", v[int(n/2)+1]"s", "min:", min"s", "max:", max"s", "n:", n }'

# Profile-guided optimization. Two-pass build that uses runtime data from a
# representative corpus to shape the final binary's branch predictor /
# inliner / register-allocator decisions. Empirically worth ~5–10% on hot
# loops; the smaller gain on register-rs (~8% at last measurement) reflects
# that the pipeline is already at the I/O floor.
#
# Usage: `just pgo <training-corpus>` — training-corpus must be a real
# directory of PBM pages. Pick a representative one (≥ ~100 pages).
pgo training_corpus="private/extracted/morimasato":
    @echo "==> 1/4 build instrumented binary"
    @rm -rf /tmp/register-pgo-data
    RUSTFLAGS="-C target-cpu=native -C force-frame-pointers=yes -Cprofile-generate=/tmp/register-pgo-data" \
        {{cargo}} build --release -p register-cli
    @echo "==> 2/4 training run on {{training_corpus}}"
    ./target/release/register {{training_corpus}} artifacts/pgo-train-out --paper b5 --dpi 300 --force
    @echo "==> 3/4 merge profiles"
    @PROFDATA=$$({{rustup}} which llvm-profdata 2>/dev/null || echo "$$HOME/.rustup/toolchains/$$(rustc --print=sysroot | xargs basename)/lib/rustlib/x86_64-unknown-linux-gnu/bin/llvm-profdata"); \
    "$$PROFDATA" merge -o /tmp/register-pgo.profdata /tmp/register-pgo-data
    @echo "==> 4/4 final build using merged profile"
    RUSTFLAGS="-C target-cpu=native -C force-frame-pointers=yes -Cprofile-use=/tmp/register-pgo.profdata" \
        {{cargo}} build --release -p register-cli
    @echo "==> PGO build done. Run \`just e2e-bench {{training_corpus}}\` to confirm gain."

# RUSTDOCFLAGS=-D warnings is also enforced in .github/workflows/docs.yml.
# This recipe matches that so pre-push catches the same drift CI would.
rustdoc-check:
    @echo "==> cargo doc --workspace --no-deps (RUSTDOCFLAGS=-D warnings)"
    @if [ "{{inside}}" = "1" ]; then \
        RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps; \
    elif [ "{{dev_running}}" = "0" ]; then \
        docker compose run --rm -e RUSTDOCFLAGS="-D warnings" dev cargo doc --workspace --no-deps; \
    else \
        docker compose exec -e RUSTDOCFLAGS="-D warnings" dev cargo doc --workspace --no-deps; \
    fi

# Aggregated lint pipeline (mirrors the CI gates that block merges).
lint: fmt-check clippy deny typos actionlint machete py-lint py-fmt-check py-typecheck

# Local CI replica.
ci: lint test rustdoc-check

# ----- git hooks -----

hooks:
    {{lefthook}} install

# ----- lefthook delegated recipes (do not run directly) -----

_hook-fmt +files:
    {{cargo}} fmt -- {{files}}

_hook-typos-fix +files:
    {{typos}} --write-changes {{files}}

_hook-taplo-fmt +files:
    {{taplo}} fmt {{files}}

_hook-cargo-sort:
    {{cargo}} sort --workspace

_hook-biome-format +files:
    {{biome}} format --write {{files}}

_hook-yamlfmt +files:
    {{yamlfmt}} {{files}}

_hook-actionlint +files:
    {{actionlint}} {{files}}
