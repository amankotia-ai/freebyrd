#!/bin/zsh
# make.sh - the Freebyrd film, end to end, as the platform film was made:
# film.html (the real notch UI, as a pure function of time) -> one texture
# sequence and camera.json per step (frames.mjs) -> one Blender file per step
# (film_step.py: the laptop round the screen) -> renders -> post.py (glow
# and grain) -> join.py -> encode.py.
#
#   CH=2 zsh film/make.sh ...           any of these for another chapter (default 1; film2.html, ch2/)
#   zsh film/make.sh snapshot          freeze ui/ for the render (edits made meanwhile don't land)
#   zsh film/make.sh tex [STEP]        texture sequences, 4x, from the snapshot
#   zsh film/make.sh blend [STEP]      the step files, saved to open in Blender
#   zsh film/make.sh preview [STEP]    every 4th frame, half size, low samples
#   zsh film/make.sh final [STEP]      every frame, full size (about 8 s a frame on an M3 Pro), in 12-frame runs
#   zsh film/make.sh reel preview|final   post, join and encode into OUT/
#   FPS=60 zsh film/make.sh tex|final|reel final   the same at 60 fps (tex60/, render60/),
#                                      highest quality: 64 samples on the screen, 32 on the laptop
#   FPS=60 zsh film/make.sh web        the small cut for pages: 1920x864, AV1 and H.264, grain in CSS
#   ENGINE=eevee FPS=60 zsh film/make.sh final|reel final|web   the same in EEVEE (eevee60/,
#                                      -eevee files, web-eevee/): about 3.8 s a frame, the look
#                                      the user preferred (2026-10-15); needs the blend step first
#
# OUT defaults to ~/Downloads/freebyrd-film-v1-blender/ch1-v2 or chN (out of iCloud).
set -e
HERE=${0:A:h}
CH=${CH:-1}                                                # the chapter (CHAPTERS.md): film.html, film2.html, ...
OUT=${OUT:-$HOME/Downloads/freebyrd-film-v1-blender/$([[ $CH == 1 ]] && echo ch1-v2 || echo ch$CH)}
PAGE=$([[ $CH == 1 ]] && echo film.html || echo film$CH.html)
BLENDER=/Applications/Blender.app/Contents/MacOS/Blender
export PATH="$HOME/.nvm/versions/node/v24.18.0/bin:$PATH"
FPS=${FPS:-30}
SUF=$([[ $FPS == 30 ]] && echo "" || echo $FPS)        # tex/ render/ at 30 fps; tex60/ render60/ at 60
ENGINE=${ENGINE:-cycles}
R=$([[ $ENGINE == eevee ]] && echo eevee$SUF || echo render$SUF)   # each step's rendered frames
ESUF=$([[ $ENGINE == eevee ]] && echo -eevee || echo "")
case $CH in                                                # window.STEPS in each chapter's page
  1) STEPS=(step1-arrive:0:6.6 step2-read:6.6:13 step3-draft:13:23.2 step4-reply:23.2:29.2 step5-promise:29.2:34.7 step6-kickoff:34.7:44 step7-booked:44:51.2) ;;
  2) STEPS=(step1-list:0:9 step2-plan:9:16 step3-adds:16:25 step4-calendar:25:31.5 step5-upnext:31.5:39.5) ;;
  3) STEPS=(step1-draft:0:8 step2-switch:8:14 step3-paste:14:20 step4-left:20:26 step5-fix:26:31.5) ;;
  4) STEPS=(step1-alert:0:7 step2-prep:7:16.6 step3-call:16.6:27.6 step4-due:27.6:35.6 step5-list:35.6:41.5) ;;
  5) STEPS=(step1-session:0:6 step2-idea:6:11.6 step3-paste:11.6:19.3 step4-work:19.3:26.6 step5-page:26.6:33.8 step6-push:33.8:42) ;;
  6) STEPS=(step1-kept:0:6.6 step2-draft:6.6:14.4 step3-send:14.4:20 step4-done:20:27 step5-checkin:27:35.5) ;;
esac

pick() { for s in $STEPS; do [[ -z $1 || ${s%%:*} == $1* ]] && echo $s; done; }

case $1 in
  snapshot)
    rm -rf $OUT/ui-snapshot && mkdir -p $OUT && cp -R $HERE/../crates/freebyrd/ui $OUT/ui-snapshot
    echo "ui frozen in $OUT/ui-snapshot" ;;
  tex)
    for s in $(pick $2); do
      name=${s%%:*}; rest=${s#*:}; a=${rest%%:*}; b=${rest#*:}
      rm -rf $OUT/$name/tex$SUF
      node $HERE/frames.mjs $OUT/$name/tex$SUF flat seq:$a:$b:$FPS --ui=$OUT/ui-snapshot --page=$PAGE | tail -1
      echo "$name: $(ls $OUT/$name/tex$SUF | grep -c png) frames"
    done ;;
  blend)
    for s in $(pick $2); do
      name=${s%%:*}
      if [[ $FPS == 30 ]]; then q=(); else q=(--samples 64 --shell-samples 32); fi
      $BLENDER -b --factory-startup -P $HERE/film_step.py -- $OUT/$name/tex$SUF $OUT/$name/render$SUF --fps $FPS $q --save $OUT/$name/$name$SUF.blend | grep -E "saved|rror"
    done ;;
  preview)
    for s in $(pick $2); do
      name=${s%%:*}
      $BLENDER -b --factory-startup -P $HERE/film_step.py -- $OUT/$name/tex $OUT/$name/preview --step 4 --res 1280x576 --samples 24 --shell-samples 8 | grep -E "rendered|rror"
    done ;;
  final)
    if [[ $ENGINE == eevee ]]; then
      # EEVEE from each step's saved file; flat on memory (3.3 GB), so 48-frame runs
      for s in $(pick $2); do
        name=${s%%:*}; n=$(ls $OUT/$name/tex$SUF | grep -c png)
        for ((f = 1; f <= n; f += 48)); do
          e=$(( f + 47 < n ? f + 47 : n )); last=$OUT/$name/$R/e-r$(printf %04d $e).png
          for try in 1 2 3; do
            (cd $HERE/.. && $BLENDER -b $OUT/$name/$name$SUF.blend -P film/eevee_frames.py -- $OUT/$name/$R e $f:$e --hq 2>&1 | grep -E "rror|Traceback") || true
            [[ -f $last ]] && break
            echo "$name $f:$e stopped early, try $try"
          done
          [[ -f $last ]] || { echo "$name $f:$e failed three times"; exit 1; }
        done
        echo "$name: $(ls $OUT/$name/$R | grep -c png) frames"
      done
      exit 0
    fi
    for s in $(pick $2); do
      name=${s%%:*}
      if [[ $FPS == 30 ]]; then q=(--persist 0); else q=(--samples 64 --shell-samples 32 --persist 0); fi
      n=$(ls $OUT/$name/tex$SUF | grep -c png)
      # a fresh Blender every 12 frames: each frame's screen is a 24-megapixel
      # image, a long run grew to 78 GB and stalled the Mac, and even short runs
      # creep about 0.3 GB a frame (2026-10-14)
      for ((f = 1; f <= n; f += 12)); do
        e=$(( f + 11 < n ? f + 11 : n )); last=$OUT/$name/render$SUF/r$(printf %04d $e).png
        for try in 1 2 3; do                  # a crash costs one retry, not the night
          $BLENDER -b --factory-startup -P $HERE/film_step.py -- $OUT/$name/tex$SUF $OUT/$name/render$SUF --fps $FPS --frames $f:$e $q | grep -E "^rendered|rror" || true
          [[ -f $last ]] && break
          echo "$name $f:$e stopped early, try $try"
        done
        [[ -f $last ]] || { echo "$name $f:$e failed three times"; exit 1; }
      done
    done ;;
  reel)
    kind=${2:-final}; src=$([[ $kind == preview ]] && echo preview || echo $R)
    dirs=()
    for s in $STEPS; do
      name=${s%%:*}
      python3 $HERE/post.py $OUT/$name/$src $OUT/$name/$src-post | tail -1
      dirs+=($OUT/$name/$src-post)
    done
    python3 $HERE/join.py $OUT/joined-$kind$SUF$ESUF $dirs
    if [[ $kind == preview ]]; then          # every 4th frame, held to real time
      ffmpeg -v error -y -framerate 7.5 -pattern_type glob -i "$OUT/joined-$kind/*.png" -r 30 -c:v libx264 -crf 18 -pix_fmt yuv420p $OUT/freebyrd-film-ch$CH-$kind.mp4
    elif [[ $FPS == 30 ]]; then
      $BLENDER -b --factory-startup -P $HERE/encode.py -- $OUT/joined-$kind $OUT/freebyrd-film-ch$CH-$kind.mp4 | grep -E "encoded|rror"
    else                                     # the master: H.264 near-lossless to watch, ProRes 422 HQ to edit
      ffmpeg -v error -y -framerate $FPS -pattern_type glob -i "$OUT/joined-$kind$SUF$ESUF/*.png" -c:v libx264 -preset slow -crf 14 -tune film -pix_fmt yuv420p -movflags +faststart $OUT/freebyrd-film-ch$CH-$FPS$ESUF.mp4
      ffmpeg -v error -y -framerate $FPS -pattern_type glob -i "$OUT/joined-$kind$SUF$ESUF/*.png" -c:v prores_ks -profile:v 3 -vendor apl0 -pix_fmt yuv422p10le $OUT/freebyrd-film-ch$CH-$FPS$ESUF-prores.mov
      ls -la $OUT/freebyrd-film-ch$CH-$FPS$ESUF*
    fi
    echo $OUT/freebyrd-film-ch$CH-$kind.mp4 ;;
  web)
    # a small clip for pages: no grain in the video (moving grain made the
    # master 663 MB); the grain goes on in CSS, as web/embed.html shows
    dirs=()
    for s in $STEPS; do
      name=${s%%:*}
      python3 $HERE/post.py $OUT/$name/$R $OUT/$name/$R-web --grain 0 | tail -1
      dirs+=($OUT/$name/$R-web)
    done
    python3 $HERE/join.py $OUT/joined-web$SUF$ESUF $dirs
    W=$OUT/web$ESUF; mkdir -p $W
    [[ $W != $OUT/web ]] && cp -n $OUT/web/embed.html $OUT/web/freebyrd-film-grain.png $W/ 2>/dev/null || true
    ffmpeg -v error -y -framerate $FPS -pattern_type glob -i "$OUT/joined-web$SUF$ESUF/*.png" -vf "scale=1920:864:flags=lanczos" -c:v libsvtav1 -preset 5 -crf 42 -g 120 -pix_fmt yuv420p -an $W/freebyrd-film-ch$CH-web.webm 2>/dev/null
    ffmpeg -v error -y -framerate $FPS -pattern_type glob -i "$OUT/joined-web$SUF$ESUF/*.png" -vf "scale=1920:864:flags=lanczos" -c:v libx264 -preset slower -crf 26 -profile:v high -pix_fmt yuv420p -movflags +faststart -an $W/freebyrd-film-ch$CH-web.mp4
    ffmpeg -v error -y -i "$OUT/joined-web$SUF$ESUF/j00001.png" -vf "scale=1920:864:flags=lanczos" -q:v 3 $W/freebyrd-film-ch$CH-poster.jpg
    ls -la $W ;;
  *) sed -n 2,18p $0 ;;
esac
