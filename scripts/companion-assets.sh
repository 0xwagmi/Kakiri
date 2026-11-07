#!/usr/bin/env bash
# Turn a clip of the companion into the animated-WebP loops Kakiri ships.
#
#   ./scripts/companion-assets.sh <source.mp4> [out-dir]
#
# The source is expected to be shot on a flat backdrop. Everything is detected:
#
#   * letterbox bars and borders, by comparing the frame's edge strips to the
#     backdrop colour,
#   * the backdrop colour, from a patch well inside the frame,
#   * the matte (see below), and
#   * the crop box, from the union of the subject's bounds across the clip.
#
# Why the matte takes two passes. A colour key alone cannot work here: the
# character's white shirt sits 0.09 away from the backdrop in RGB and her skin
# 0.07, so any key wide enough to clean the backdrop also erased her clothes and
# skin -- which is exactly what happened with colorkey at similarity 0.10. So:
#
#   1. flood fill from the frame edges, which removes the background as a
#      connected region -- gradient, floor shadow and all -- and cannot cross her
#      dark outline, so shirt and skin survive;
#   2. a narrow colour test for the pockets the fill cannot reach (background
#      trapped between her legs is not connected to the edge), done with
#      -transparent because it only ever clears alpha -- colorkey would *set*
#      alpha, turning the pixels the fill had just cleared back into an opaque
#      black box around her.
#
# Segments are given in SEGMENTS as "name:start:seconds[:palindrome]". Every
# state comes from its own clip, so run the script once per clip:
#
#   SEGMENTS="wave:0:5.3" ./scripts/companion-assets.sh ~/Videos/kakiri-companion/wave_hello.mp4
#
# REF_FRAC pins the scale to a fraction of frame height (ours: 0.668, the standing
# body in these clips). Clips shot with the same camera must share it, or a seated
# state gets blown up to standing height.
#
# `palindrome` plays the loop half backwards, which turns a one-way transition
# into something that breathes instead of snapping.
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
WORK_H="${WORK_H:-480}"
FUZZ="${FUZZ:-8}"
# trapped background is exactly the backdrop colour; her shirt sits ~9% away and
# skin ~7%, so keeping this well under that is what protects them
POCKET_FUZZ="${POCKET_FUZZ:-4}"
REF_FRAC="${REF_FRAC:-}"

command -v ffmpeg >/dev/null || { echo "companion-assets: ffmpeg is required" >&2; exit 1; }
command -v convert >/dev/null || { echo "companion-assets: imagemagick is required" >&2; exit 1; }
[ -f "$SRC" ] || { echo "companion-assets: no such file: $SRC" >&2; exit 1; }
mkdir -p "$OUT"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

fw=$(ffprobe -v error -select_streams v:0 -show_entries stream=width -of csv=p=0 "$SRC" | cut -d, -f1)
fh=$(ffprobe -v error -select_streams v:0 -show_entries stream=height -of csv=p=0 "$SRC" | cut -d, -f1)
probe="$work/probe.png"
ffmpeg -v error -ss "$PROBE_AT" -i "$SRC" -frames:v 1 "$probe" -y

# --- backdrop colour ------------------------------------------------------
# 16x16 patch 8% inside the frame: an AI backdrop carries noise, and the very
# edge is where generators leave borders.
read -r R G B < <(ffmpeg -v error -i "$probe" \
  -vf "crop=16:16:floor(iw*0.08):floor(ih*0.08),scale=1:1:flags=area,format=rgb24" -f rawvideo - | od -An -tu1 -N3)
hex=$(printf '0x%02X%02X%02X' "$R" "$G" "$B")
imhex=$(printf '#%02X%02X%02X' "$R" "$G" "$B")

# --- margins the generator added -----------------------------------------
# Generated clips arrive with a black bar or a floor gradient at the bottom (ours
# had ~200 px). Left in place it drags the crop box out to the frame edge and
# shrinks the character, so walk in from each edge until the strip matches the
# backdrop. The strips are taken at the frame border, where the subject is not.
margins() { # $1 = crop+scale filter, $2 = length
  ffmpeg -v error -i "$probe" -vf "$1,format=rgb24" -f rawvideo - | od -An -tu1 -v \
    | tr -s ' \n' '\n' | paste - - - | awk -v R="$R" -v G="$G" -v B="$B" -v n="$2" '
      { d = ($1-R)^2 + ($2-G)^2 + ($3-B)^2; if (d < 1200) { if (f == "") f = NR; l = NR } }
      END { if (f == "") { print 0, n - 1 } else print f - 1, n - l }'
}
read -r TOP BOT < <(margins "crop=iw*0.12:ih:0:0,scale=1:ih:flags=area" "$fh")
read -r LEFT RIGHT < <(margins "crop=iw:ih*0.12:0:0,scale=iw:1:flags=area" "$fw")

T=$(( TOP + INSET )); Bm=$(( BOT + INSET ))
L=$(( LEFT + INSET )); Rr=$(( RIGHT + INSET ))
[ $(( T + Bm )) -gt $(( fh / 2 )) ] && { T=0; Bm=0; }
[ $(( L + Rr )) -gt $(( fw / 2 )) ] && { L=0; Rr=0; }

trim="crop=iw-${L}-${Rr}:ih-${T}-${Bm}:${L}:${T}"
echo "source:   $(basename "$SRC")  ${fw}x${fh} -> ${hex} backdrop, edges -${L}/-${Rr}/-${T}/-${Bm}"

# --- one loop per segment ------------------------------------------------
for segment in $SEGMENTS; do
  IFS=: read -r name start length effect <<<"$segment"
  dir="$work/$name"; mkdir -p "$dir"

  # 1. decode the segment at working resolution
  ffmpeg -v error -y -ss "$start" -t "$length" -i "$SRC" \
    -vf "${trim},fps=${FPS},scale=-2:${WORK_H}:flags=lanczos" "$dir/raw_%04d.png"

  # 2. flood fill each frame from its edges
  first=$(ls "$dir"/raw_*.png | head -1)
  W=$(identify -format "%w" "$first"); H=$(identify -format "%h" "$first")
  seeds=()
  for y in 0 $((H-1)); do
    for x in 0 $((W/4)) $((W/2)) $((W*3/4)) $((W-1)); do
      seeds+=(-draw "color $x,$y floodfill")
    done
  done
  for x in 0 $((W-1)); do
    for y in $((H/4)) $((H/2)) $((H*3/4)); do
      seeds+=(-draw "color $x,$y floodfill")
    done
  done
  for f in "$dir"/raw_*.png; do
    convert "$f" -alpha set -fuzz "${FUZZ}%" -fill none "${seeds[@]}" \
      -fuzz "${POCKET_FUZZ}%" -transparent "$imhex" \
      -channel A -blur 0x0.6 -level 45%,55% +channel "${f%.png}_key.png"
  done

  # 3. subject bounds: union for the crop, median height for the scale
  for f in "$dir"/*_key.png; do
    convert "$f" -alpha extract -threshold 20% -format "%@\n" info:
  done > "$dir/bounds.txt"
  union=$(awk -F'[x+]' '
      { if (NR == 1 || $3 < x0) x0 = $3; if (NR == 1 || $4 < y0) y0 = $4
        if ($1 + $3 > x1) x1 = $1 + $3; if ($2 + $4 > y1) y1 = $2 + $4 }
      END { printf "%d %d %d %d", x0, y0, x1 - x0, y1 - y0 }' "$dir/bounds.txt")
  read -r bx by bw bh <<<"$union"
  medh=$(cut -d'x' -f2 "$dir/bounds.txt" | cut -d+ -f1 | sort -n          | awk '{a[NR]=$1} END { print a[int((NR + 1) / 2)] }')

  cw=$(( (bw + 2*PAD) / 2 * 2 )); ch=$(( (bh + 2*PAD) / 2 * 2 ))
  cx=$(( bx - PAD )); cy=$(( by - PAD ))
  [ "$cx" -lt 0 ] && cx=0
  [ "$cy" -lt 0 ] && cy=0
  [ $((cx + cw)) -gt "$W" ] && cw=$(( (W - cx) / 2 * 2 ))
  [ $((cy + ch)) -gt "$H" ] && ch=$(( (H - cy) / 2 * 2 ))

  # 4. encode; the narrow key clears background the fill could not reach
  if [ -n "$REF_FRAC" ]; then
    ref=$(awk -v f="$REF_FRAC" -v h="$WORK_H" 'BEGIN { printf "%d", f*h }')
  else
    ref=$medh
  fi
  out_h=$(awk -v ch="$ch" -v m="$ref" -v t="$HEIGHT" 'BEGIN { printf "%d", ch*t/m/2*2 }')
  vf="crop=${cw}:${ch}:${cx}:${cy},scale=-2:${out_h}:flags=lanczos,format=yuva420p"
  if [ "${effect:-}" = "palindrome" ]; then
    ffmpeg -v error -y -framerate "$FPS" -i "$dir/raw_%04d_key.png" \
      -filter_complex "${vf},split[fwd][rev];[rev]reverse[bwd];[fwd][bwd]concat=n=2:v=1[out]" \
      -map "[out]" -c:v libwebp_anim -q:v "$QUALITY" -loop 0 -an "${OUT}/${name}.webp"
  else
    ffmpeg -v error -y -framerate "$FPS" -i "$dir/raw_%04d_key.png" \
      -vf "$vf" -c:v libwebp_anim -q:v "$QUALITY" -loop 0 -an "${OUT}/${name}.webp"
  fi

  printf '  %-8s %4.1fs  %-8s %-6s crop %sx%s\n' "$name" "$length" \
    "$(identify -format '%wx%h' "${OUT}/${name}.webp" | head -1)" \
    "$(du -h "${OUT}/${name}.webp" | cut -f1)" "$cw" "$ch"
done
