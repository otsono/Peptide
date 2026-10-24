# Peptide task runner. run `just` (no args) to list recipes.
# install just: https://github.com/casey/just  (brew install just)

# corpus location: $SSF2_SSFS_DIR if set, else the default sibling ../ssf2-ssfs
ssfs := env_var_or_default("SSF2_SSFS_DIR", "../ssf2-ssfs")

# list recipes
default:
    @just --list

# build the release binary -> build/release/peptide
build:
    cargo build --release

# run the whole test suite (corpus tests skip cleanly if the corpus is absent)
test:
    cargo test --workspace

# the same clippy gate CI enforces
lint:
    cargo clippy --workspace -- -D warnings

# format the whole codebase (heads up: large one-time diff -- never been run)
fmt:
    cargo fmt

# check formatting without writing (advisory; not yet CI-enforced)
fmt-check:
    cargo fmt -- --check

# convert one character by id, e.g. `just convert mario`
convert CHAR:
    ./build/release/peptide convert "{{ssfs}}/{{CHAR}}.ssf"

# fast inner loop: rebuild + reconvert sandbag
smoke:
    ./tools/rebuild-sandbag.sh

# regression sweep after stage-converter changes: a fixed SAMPLE, chosen for coverage.
#
# Not the whole corpus. Converting all 110 takes hours and writes tens of gigabytes -- one stage
# alone emits 687MB -- so nobody runs it, and a gate nobody runs is not a gate. These ten cover
# the paths a stage change can break: plain geometry, hazards declared in initialize and in
# update, weather, moving platforms, backdrop elements, and boundary/parallax handling. Every one
# is small, so the whole sweep is quick enough to run before every PR.
#
# There is deliberately NO full-corpus convert recipe: it takes hours and writes tens of
# gigabytes. The sample is the regression gate. To find out how widespread something is,
# SCAN with `peptide ssf2 stage <f> --info`, which parses without emitting.
sample_stages := "battlefield battlefield2 bombfactory butterbuilding bowserscastle crateria crystalsmash flatzoneplus fourside huecomundo"

sweep-stages:
    #!/usr/bin/env bash
    set -u
    fails=0; total=0
    for name in {{sample_stages}}; do
        total=$((total+1))
        if ! ./build/release/peptide ssf2 stage "{{ssfs}}/stages/$name.ssf" --out build/sweep >/dev/null 2>&1; then
            echo "FAIL: $name"; fails=$((fails+1))
        fi
    done
    echo "$((total-fails))/$total sample stages converted clean"
    # CONVERTING IS NOT THE WHOLE BAR. The engine drops a script it cannot parse WHOLE, so a
    # stage that converts clean can still lose every object it makes over one stray bracket --
    # silently, with the first symptom a frame reporting an unknown variable for a name that is
    # right there in the file. This reads what was emitted and refuses the shapes that do that.
    python3 tools/check-emitted.py build/sweep || fails=$((fails+1))
    exit $fails

# the converter's floor for the whole roster, offline: every fighter converted in parallel, its
# emitted scripts checked for parse problems and calls nothing answers, and what it leaves unported
# counted per kind against tools/tests/strike_tally.json. minutes; `-- --update` keeps a new floor
converter-audit *args:
    python3 tools/tests/strike_tally.py {{args}}

# the same ten, but RUN: a stage that converts can still throw the moment its scripts tick
sweep-stages-live:
    python3 tools/verify.py stage {{sample_stages}}

# THE BAR FOR "PORTED": ten stages and ten fighters converted, published, booted and driven,
# each held to the plan the converter wrote by what its own running script reports. hours.
verify:
    python3 tools/verify.py sample

# every fighter's six gates on the runtime's path, from a frozen build: `just verify-roster`, or
# name some (`just verify-roster mario sonic`). hours for the whole roster; results in
# build/verify/roster.txt
verify-roster *FIGHTERS:
    ./tools/verify-roster.sh {{FIGHTERS}}

# each fighter's hits landed on both engines, damage and launch side by side (aerials included):
# `just hits-roster`, or one fighter (`just hits-roster mario`). results in build/verify/hits-<id>.json
hits-roster *FIGHTERS:
    PEPTIDE_RUNTIME=1 python3 tools/tests/hit_parity.py --all {{FIGHTERS}}

# one subject through every gate: `just verify-one stage fourside` / `just verify-one char mario`
verify-one KIND ID *ARGS:
    python3 tools/verify.py {{KIND}} {{ID}} {{ARGS}}

dump BIN *ARGS:
    cargo run -p ssf2_converter --features dev-tools --bin {{BIN}} -- {{ARGS}}

# build the double-clickable macOS app -> build/Peptide.app
app:
    ./tools/make-app.sh
