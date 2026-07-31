#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

die() {
	echo "hntui screenshots: $*" >&2
	exit 1
}

font_family="CaskaydiaCove Nerd Font Mono"

command -v vhs >/dev/null 2>&1 || die "missing command: vhs"
command -v fc-match >/dev/null 2>&1 || die "missing command: fc-match"
command -v magick >/dev/null 2>&1 || die "missing command: magick"

# Robot, YouTube, and Codeberg in current Nerd Fonts.
for codepoint in EE0D F16A F330; do
	resolved_font="$(fc-match -f '%{family[0]}' "${font_family}:charset=${codepoint}")"
	[[ "$resolved_font" == "$font_family" ]] ||
		die "install ${font_family} with required glyph U+${codepoint}"
done

vhs validate 'scripts/screenshot-*.tape'
capture_started_at="$(mktemp "${TMPDIR:-/tmp}/hntui-screenshots.XXXXXX")"
trap 'rm -f "$capture_started_at"' EXIT

cargo build --release --locked

echo "Capturing demo GIF..."
vhs scripts/screenshot-demo.tape

echo "Capturing stories view..."
vhs scripts/screenshot-stories.tape

echo "Capturing comments view..."
vhs scripts/screenshot-comments.tape

for output in screenshots/demo.gif screenshots/stories.png screenshots/comments.png; do
	[[ -s "$output" && "$output" -nt "$capture_started_at" ]] ||
		die "output was not regenerated: $output"
done

for screenshot in screenshots/stories.png screenshots/comments.png; do
	colors="$(magick identify -format '%k' "$screenshot")"
	((colors > 1)) || die "screenshot is blank: $screenshot"

	chromatic_fraction="$(
		magick "$screenshot" -colorspace HSL -channel G -separate +channel \
			-threshold 10% -format '%[fx:mean]' info:
	)"
	awk -v value="$chromatic_fraction" 'BEGIN { exit !(value >= 0.05) }' ||
		die "screenshot lost its color palette: $screenshot"
done

echo "Done. Screenshots saved to screenshots/"
ls -lh screenshots/*.png screenshots/*.gif
