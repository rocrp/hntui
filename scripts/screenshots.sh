#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release

echo "Capturing demo GIF..."
vhs scripts/screenshot-demo.tape

echo "Capturing stories view..."
vhs scripts/screenshot-stories.tape

echo "Capturing comments view..."
vhs scripts/screenshot-comments.tape

echo "Done. Screenshots saved to screenshots/"
ls -lh screenshots/*.png screenshots/*.gif
