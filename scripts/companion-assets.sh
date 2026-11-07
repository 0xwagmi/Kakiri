#!/usr/bin/env bash
# Turn a clip of the companion into the animated-WebP loops Kakiri ships.
#
#   ./scripts/companion-assets.sh <source.mp4> [out-dir]
#
# The source is expected to be shot on a flat backdrop. Everything else is
# detected rather than configured:
#
#   * letterbox bars and edge borders, by comparing the frame's edge strips to
#     the backdrop colour,
#   * the backdrop colour itself, from a patch well inside the frame,
#   * the crop box, from the alpha of the keyed subject (union of every pose).
#
# Segments are given in SEGMENTS as "name:start:seconds[:palindrome]". Every
# state comes from its own clip, so run the script once per clip:
#
#   SEGMENTS="wave:0:5.3" ./scripts/companion-assets.sh ~/Videos/kakiri-companion/wave_hello.mp4
#
# `palindrome` plays the loop half backwards, which turns a one-way transition
# (standing -> crouch) into something that breathes instead of snapping.
set -euo pipefail

SRC="${1:?usage: companion-assets.sh <source.mp4> [out-dir]}"
OUT="${2:-src/lib/assets/companion}"
SEGMENTS="${SEGMENTS:-dance:0:14.6}"
HEIGHT="${HEIGHT:-240}"
FPS="${FPS:-15}"
QUALITY="${QUALITY:-78}"
PAD="${PAD:-8}"
INSET="${INSET:-2}"
PROBE_AT="${PROBE_AT:-1.5}"

command -v ffmpeg >/dev/null || { echo "companion-assets: ffmpeg is required" >&2; exit 1; }
[ -f "$SRC" ] || { echo "companion-assets: no such file: $SRC" >&2; exit 1; }
mkdir -p "$OUT"

fw=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$SRC" | cut -d, -f1)
fh=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$SRC" | cut -d, -f1)
echo "source:   $(basename "$SRC")  ${fw}x${fh}"

probe=$(mktemp --suffix=.png)
trap 'rm -f "$probe"' EXIT
ffmpeg -v error -ss "$PROBE_AT" -i "$SRC" -frames:v 1 "$probe" -y

# --- backdrop colour ------------------------------------------------------
# Sampled 8% inside the frame: an AI backdrop carries a little noise, so a single
# pixel drifts, and the very edge is where generators leave borders.
read -r R G B < <(
  ffmpeg -v error -i "$probe" -vf "crop=16:16:floor(iw*0.08):floor(ih*0.08),scale=1:1:flags=area,format=rgb24" \
    -f rawvideo - | od -An -tu1 -N3)
hex=$(printf '0x%02X%02X%02X' "$R" "$G" "$B")
echo "backdrop: $hex"

# --- margins the generator added -----------------------------------------
# Generated clips arrive with a black bar (ours had ~200 px of floor gradient and
# black at the bottom) or a 1 px border. None of that is backdrop; left in place
# it never keys out, and it drags the crop box out to the frame edge, which
# shrinks the character. So walk in from each edge until the strip matches the
# backdrop. The strips are taken at the frame's border, where the subject is not,
# so she can never extend the margins.
margins() { # $1 = crop+scale filter, $2 = supposed length
  ffmpeg -v error -i "$probe" -vf "$1,format=rgb24" -f rawvideo - | od -An -tu1 -v \
    | tr -s ' \n' '\n' | paste - - - | awk -v R="$R" -v G="$G" -v B="$B" -v n="$2" '
      { d = ($1-R)^2 + ($2-G)^2 + ($3-B)^2; if (d < 1200) { if (f == "") f = NR; l = NR } }
      END { if (f == "") { print 0, n - 1 } else print f - 1, n - l }'
}
read -r TOP BOT < <(margins "crop=iw*0.12:ih:0:0,scale=1:ih:flags=area" "$fh")
read -r LEFT RIGHT < <(margins "crop=iw:ih*0.12:0:0,scale=iw:1:flags=area" "$fw")

# a hair of extra inset, and never trust a profile that eats most of the frame
T=$(( TOP + INSET )); Bm=$(( BOT + INSET ))
L=$(( LEFT + INSET )); Rr=$(( RIGHT + INSET ))
[ $(( T + Bm )) -gt $(( fh / 2 )) ] && { T=0; Bm=0; }
[ $(( L + Rr )) -gt $(( fw / 2 )) ] && { L=0; Rr=0; }

trim="crop=iw-${L}-${Rr}:ih-${T}-${Bm}:${L}:${T}"
fw=$(( fw - L - Rr )); fh=$(( fh - T - Bm ))
echo "edges:    -${L}/-${Rr} left-right, -${T}/-${Bm} top-bottom -> ${fw}x${fh}"

# --- crop box, from the alpha of the keyed subject -----------------------
# alphaextract turns the matte into luma and cropdetect finds its bounds, so the
# subject does not have to sit in the middle of the frame. reset=0 accumulates
# over the whole clip, giving the union of every pose: one box for all frames, so
# she neither changes size between poses nor gets clipped mid-gesture.
# `format=yuva420p` is required -- colorkey alone does not negotiate an alpha
# format and alphaextract dies with "Error reinitializing filters".
crop=$(
  ffmpeg -hide_banner -nostats -loglevel info -i "$SRC" \
    -vf "${trim},colorkey=${hex}:0.10:0.02,format=yuva420p,alphaextract,cropdetect=limit=0.02:round=2:reset=0" \
    -f null - 2>&1 \
  | sed -n 's/.*crop=\([0-9]*:[0-9]*:[0-9]*:[0-9]*\).*/\1/p' | tail -1 || true)

if [ -n "${crop:-}" ]; then
  IFS=: read -r cw ch cx cy <<<"$crop"
else
  echo "companion-assets: cropdetect found nothing, using the whole frame" >&2
  cw=$fw; ch=$fh; cx=0; cy=0
fi

cw=$(( (cw + 2*PAD) / 2 * 2 )); ch=$(( (ch + 2*PAD) / 2 * 2 ))
cx=$(( cx - PAD )); cy=$(( cy - PAD ))
[ "$cx" -lt 0 ] && cx=0
[ "$cy" -lt 0 ] && cy=0
[ $((cx + cw)) -gt "$fw" ] && cw=$(( (fw - cx) / 2 * 2 ))
[ $((cy + ch)) -gt "$fh" ] && ch=$(( (fh - cy) / 2 * 2 ))
# --- how tall is she, really -----------------------------------------------
# The crop box is the union of every pose, so a clip with a raised arm gets a
# taller box than a clip without one -- normalising by that box height would make
# her grow and shrink between states. The median subject height over all frames
# is the stable measure, so scale that to HEIGHT. By overriding SCALE_H /
# SCALE_W a state can opt out (a seated or lying pose is supposed to be short).
median_h=$(
  ffmpeg -hide_banner -nostats -loglevel info -i "$SRC" \
    -vf "${trim},colorkey=${hex}:0.10:0.02,format=yuva420p,alphaextract,cropdetect=limit=0.02:round=2:reset=1" \
    -f null - 2>&1 \
  | sed -n 's/.*crop=[0-9]*:\([0-9]*\):.*/\1/p' | sort -n \
  | awk '{a[NR]=$1} END { if (NR == 0) print 0; else print a[int((NR+1)/2)] }')

# REF_H pins the scale to a reference measurement instead of this clip's own
# median: clips shot with the same camera should all use one number, or a seated
# state would be blown up to standing height.
ref=${REF_H:-$median_h}

if [ -n "${SCALE_W:-}" ]; then
  geom="${SCALE_W}:-2"
elif [ -n "${SCALE_H:-}" ]; then
  geom="-2:${SCALE_H}"
elif [ "${ref:-0}" -gt 0 ]; then
  out_h=$(awk -v ch="$ch" -v m="$ref" -v t="$HEIGHT" 'BEGIN { printf "%d", ch*t/m/2*2 }')
  geom="-2:${out_h}"
else
  geom="-2:${HEIGHT}"
fi
echo "subject:  ${cw}x${ch}+${cx}+${cy}  median ${median_h}px ref ${ref}px -> scale ${geom}"

# --- one loop per segment ------------------------------------------------
base="${trim},colorkey=${hex}:0.10:0.02,format=yuva420p,crop=${cw}:${ch}:${cx}:${cy},scale=${geom}:flags=lanczos,fps=${FPS}"

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
  printf '  %-8s %4.1fs  %-8s %s\n' "$name" "$length" \
    "$(identify -format '%wx%h' "$target" 2>/dev/null | head -1)" "$(du -h "$target" | cut -f1)"
done
