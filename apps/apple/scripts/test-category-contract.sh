#!/bin/bash
set -euo pipefail
apple=$(cd "$(dirname "$0")/.." && pwd)
node "$apple/scripts/test-category-contract.mjs" "$apple"
