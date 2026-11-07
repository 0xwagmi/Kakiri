#!/usr/bin/env bash
# Turn a clip of the companion into the animated-WebP loops Kakiri ships.
#
#   ./scripts/companion-assets.sh <source.mp4> [out-dir]
#
# The source is expected to be shot on a flat backdrop (any colour: grey, green,
# white). Two things are detected rather than configured:
#
#   * the backdrop colour, sampled from a corner, and
#   * the crop box, from the alpha of a keyed frame.
#
# Segments below are cut from Kakiri's own clip; with different footage, adjust
# SEGMENTS. Each entry is "name:start:seconds[:palindrome]". `palindrome` plays
# the loop half backwards, which turns a one-way transition (standing -> crouch)
# into something that breathes instead of snapping.
set -euo pipefail

SRC="${1:?usage: companion-assets.sh <source.mp4> [out-dir]}"
OUT="${2:-src/lib/assets/companion}"
# One loop: the whole dance, cut at 14.6 s rather than 16.4 s because the clip
# ends in a crouch and looping that back to a standing frame reads as a glitch.
# Frame 14.6 has the same arms-out pose as frame 0, so the seam is invisible.
SEGMENTS="${SEGMENTS:-dance:0:14.6}"
HEIGHT="${HEIGHT:-240}"
FPS="${FPS:-15}"
PAD="${PAD:-8}"
# Generated clips often carry a 1 px border in a different colour (one of ours came
# with a black left edge). It never keys out, and cropdetect then believes the
# subject spans the whole frame -- so trim the edges before anything else.
INSET="${INSET:-4}"
QUALITY="${QUALITY:-78}"

command -v ffmpeg >/dev/null || { echo "companion-assets: ffmpeg is required" >&2; exit 1; }
[ -f "$SRC" ] || { echo "companion-assets: no such file: $SRC" >&2; exit 1; }
mkdir -p "$OUT"

echo "source:   $SRC"

# --- backdrop colour, sampled from a corner -------------------------------
# A 16x16 patch averaged to one pixel: AI-video backdrops carry a little noise,
# so a single-pixel reading drifts from frame to frame.
hex=$(ffmpeg -v error -i "$SRC" -frames:v 1 \
        -vf "crop=iw-2*${INSET}:ih-2*${INSET}:${INSET}:${INSET},crop=16:16:0:0,scale=1:1,format=rgb24" -f rawvideo - \
      | od -An -tu1 -N3 | awk '{printf "0x%02X%02X%02X", $1, $2, $3}')
[ -n "$hex" ] || { echo "companion-assets: could not sample the backdrop" >&2; exit 1; }
echo "backdrop: $hex"

# --- crop box, from the alpha of the keyed subject ------------------------
# alphaextract turns the matte into luma and cropdetect finds its bounds, so the
# subject does not have to sit in the middle of the frame. reset=0 makes the
# detector accumulate over the whole clip, giving the union of every pose: one
# box for all states, so she neither changes size between them nor gets clipped
# mid-gesture. `format=yuva420p` is required -- colorkey alone does not negotiate
# an alpha format here and alphaextract fails with "Error reinitializing
# filters". cropdetect reports at info level, so drop -v error for this pass.
crop=$(
  ffmpeg -hide_banner -nostats -loglevel info -i "$SRC" \
    -vf "crop=iw-2*${INSET}:ih-2*${INSET}:${INSET}:${INSET},colorkey=${hex}:0.10:0.02,format=yuva420p,alphaextract,cropdetect=limit=0.02:round=2:reset=0" \
    -f null - 2>&1 \
  | sed -n 's/.*crop=\([0-9]*:[0-9]*:[0-9]*:[0-9]*\).*/\1/p' | tail -1 || true)
if [ -n "$crop" ]; then
  IFS=: read -r cw ch cx cy <<<"$crop"
else
  echo "companion-assets: cropdetect found nothing, using the whole frame" >&2
  IFS=, read -r cw ch _ <<<"$(ffprobe -v error -select_streams v:0 \
    -show_entries stream=width,height -of csv=p=0 "$SRC")"
  cx=0 cy=0
fi

# a little air around the union box, clamped to the frame and kept even
fw=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$SRC" | cut -d, -f1)
fh=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$SRC" | cut -d, -f1)
fw=$((fw - 2*INSET)); fh=$((fh - 2*INSET))
cw=$(( (cw + 2*PAD) / 2 * 2 )); ch=$(( (ch + 2*PAD) / 2 * 2 ))
cx=$(( cx - PAD )); cy=$(( cy - PAD ))
[ "$cx" -lt 0 ] && cx=0
[ "$cy" -lt 0 ] && cy=0
[ $((cx + cw)) -gt "$fw" ] && cw=$(( (fw - cx) / 2 * 2 ))
[ $((cy + ch)) -gt "$fh" ] && ch=$(( (fh - cy) / 2 * 2 ))
echo "subject:  ${cw}x${ch}+${cx}+${cy}  (scaled to ${HEIGHT}px tall)"

# --- one loop per segment ------------------------------------------------
# scale by height: she is tall and narrow, so a fixed width would blow her up
base="crop=iw-2*${INSET}:ih-2*${INSET}:${INSET}:${INSET},colorkey=${hex}:0.10:0.02,format=yuva420p,crop=${cw}:${ch}:${cx}:${cy},scale=-2:${HEIGHT}:flags=lanczos,fps=${FPS}"

for segment in $SEGMENTS; do
  IFS=: read -r name start length effect <<<"$segment"
  target="${OUT}/${name}.webp"
  if [ "${effect:-}" = "palindrome" ]; then
    ffmpeg -v error -y -ss "$start" -t "$length" -i "$SRC" \
      -filter_complex "${base},split[fwd][rev];[rev]reverse[bwd];[fwd][bwd]concat=n=2:v=1[out]" \
      -map "[out]" -c:v libwebp_anim -q:v "$QUALITY" -loop 0 -an "${target}"
  else
    ffmpeg -v error -y -ss "$start" -t "$length" -i "$SRC" \
      -vf "${base}" -c:v libwebp_anim -q:v "$QUALITY" -loop 0 -an "${target}"
  fi
  printf '  %-6s %.1fs  %s\n' "$name" "$length" "$(du -h "$target" | cut -f1)"
done

echo
echo "wrote $(ls -1 "$OUT" | wc -l) files to $OUT ($(du -sh "$OUT" | cut -f1))"
