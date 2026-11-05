#!/usr/bin/env bash
# Build and ship the browser version to https://monet.hunterharloff.com (Vercel).
set -euo pipefail
cd "$(dirname "$0")"
./build.sh
cp vercel.json dist/
mkdir -p .vercel && [ -f .vercel/project.json ] || (cd dist && vercel link --yes --project monet ${VERCEL_SCOPE:+--scope "$VERCEL_SCOPE"} && cp -r .vercel ..)
cp -r .vercel dist/
cd dist && vercel deploy --prod --yes
