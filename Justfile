mod? local

fmt *args:
    ./tools/dprint.dotslash fmt {{args}}

lint *args:
    cargo clippy --fix --allow-dirty --allow-staged --all-features --all-targets {{args}}



