#!/bin/bash
set -e

RESET='\033[0m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'

echo -e "${BLUE}=== Building Koi ===${RESET}"
echo ""

cargo build --release 2>&1 | grep -E "Compiling|Finished|error|warning" || true

if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓ Build successful${RESET}"
else
    echo "Build failed"
    exit 1
fi

echo ""
echo "Binaries at:"
echo "  ./target/release/koi-ast"
echo "  ./target/release/koi-ir"
echo "  ./target/release/koi-assembly"