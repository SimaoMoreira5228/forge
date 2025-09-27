mod? local

fmt *args:
    dprint fmt {{args}}

lint *args:
    cargo clippy --fix --allow-dirty --allow-staged --all-features --all-targets {{args}}



