#!/usr/bin/env bash
# Authenticated, headless WRDP compatibility probe.
# The password is read from WRDP_TEST_PASSWORD or stdin and is never passed as
# an argument or written to the result directory.
set -euo pipefail

TARGET=${WRDP_TEST_TARGET:-127.0.0.1:3389}
USER_NAME=${WRDP_TEST_USER:-rui}
HOLD_SECONDS=${WRDP_TEST_HOLD_SECONDS:-20}
OUTPUT=${WRDP_TEST_OUTPUT:-"$(pwd)/.local/rdp-compat/$(date +%Y%m%d-%H%M%S)"}
DISPLAY_NUMBER=${WRDP_TEST_DISPLAY:-:97}
CLIENT=${WRDP_TEST_CLIENT:-xfreerdp3}
RECONNECTS=${WRDP_TEST_RECONNECTS:-3}
RECONNECT_SPACING=${WRDP_TEST_RECONNECT_SPACING:-5}
SIZES=(864x635 1376x960 1280x720 1920x1080)

mkdir -p "$OUTPUT/raw"
chmod 700 "$OUTPUT" "$OUTPUT/raw"

if [[ -n ${WRDP_TEST_PASSWORD:-} ]]; then
    password=$WRDP_TEST_PASSWORD
else
    IFS= read -r -s password
fi
[[ -n $password ]] || { echo "missing WRDP test password" >&2; exit 2; }
command -v "$CLIENT" >/dev/null
command -v Xvfb >/dev/null

Xvfb "$DISPLAY_NUMBER" -screen 0 2200x1400x24 -nolisten tcp >"$OUTPUT/raw/xvfb.log" 2>&1 &
xvfb_pid=$!
cleanup() {
    kill "$xvfb_pid" 2>/dev/null || true
    unset password WRDP_TEST_PASSWORD
}
trap cleanup EXIT INT TERM
sleep 1
export DISPLAY=$DISPLAY_NUMBER
h264_build=$($CLIENT /buildconfig 2>/dev/null | grep -q 'WITH_GFX_H264=ON' && echo yes || echo no)
printf 'client_h264=%s\n' "$h264_build" >"$OUTPUT/environment.txt"
printf 'mode\tsize\trc\tduration_s\tactive\tgfx_confirm\tsurface\tavc420\tdecode_error\tdeactivate\terror_200d\n' >"$OUTPUT/results.tsv"

run_case() {
    local mode=$1 size=$2 rc start duration
    local log="$OUTPUT/raw/${mode}-${size}.log"
    local -a graphics
    case "$mode" in
        gfx) graphics=(/gfx) ;;
        bitmap) graphics=(-gfx /gdi:sw) ;;
        rfx) graphics=(/gfx:RFX:on) ;;
        progressive) graphics=(/gfx:progressive:on) ;;
        *) return 2 ;;
    esac
    start=$(date +%s)
    set +e
    printf '%s\n' "$password" | timeout "$HOLD_SECONDS" "$CLIENT" \
        "/v:$TARGET" "/u:$USER_NAME" /from-stdin:force /cert:ignore "/size:$size" \
        "${graphics[@]}" /network:lan /timeout:10000 /log-level:TRACE >"$log" 2>&1
    rc=$?
    set -e
    duration=$(($(date +%s)-start))
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' \
        "$mode" "$size" "$rc" "$duration" \
        "$(grep -c 'CONNECTION_STATE_ACTIVE' "$log" || true)" \
        "$(grep -c 'CAPSCONFIRM' "$log" || true)" \
        "$(grep -cE 'CREATE_SURFACE|CreateSurface' "$log" || true)" \
        "$(grep -c 'CODECID_AVC420' "$log" || true)" \
        "$(grep -c 'decode_AVC420 failed' "$log" || true)" \
        "$(grep -ciE 'PDU_TYPE_DEACTIVATE_ALL|reactiv' "$log" || true)" \
        "$(grep -ciE '0x0002000D|0x0000200D|0x200d' "$log" || true)" >>"$OUTPUT/results.tsv"
}

for size in "${SIZES[@]}"; do run_case gfx "$size"; done
for size in "${SIZES[@]}"; do run_case bitmap "$size"; done
run_case rfx 1280x720
run_case progressive 1280x720

for attempt in $(seq 1 "$RECONNECTS"); do
    run_case "gfx" 1280x720
    sleep "$RECONNECT_SPACING"
done

if [[ ${WRDP_TEST_DYNAMIC:-0} == 1 ]]; then
    if command -v openbox >/dev/null && command -v xdotool >/dev/null; then
        openbox >"$OUTPUT/raw/openbox.log" 2>&1 &
        openbox_pid=$!
        printf '%s\n' "$password" | timeout $((HOLD_SECONDS + 40)) "$CLIENT" \
            "/v:$TARGET" "/u:$USER_NAME" /from-stdin:force /cert:ignore /size:1280x720 \
            /gfx +dynamic-resolution /wm-class:WRDPCompatProbe /network:lan /timeout:10000 \
            /log-level:TRACE >"$OUTPUT/raw/dynamic.log" 2>&1 &
        dynamic_pid=$!
        window=''
        for _ in $(seq 1 20); do
            window=$(xdotool search --class WRDPCompatProbe 2>/dev/null | head -1 || true)
            [[ -n $window ]] && break
            sleep 1
        done
        if [[ -n $window ]]; then
            for dimensions in '864 635' '1376 960' '1280 720' '1920 1080'; do
                read -r width height <<<"$dimensions"
                printf '%s\t%sx%s\n' "$(date -Is)" "$width" "$height" >>"$OUTPUT/dynamic-events.tsv"
                xdotool windowsize "$window" "$width" "$height"
                sleep 8
            done
        else
            printf 'dynamic\tSKIP\twindow-not-found\n' >"$OUTPUT/dynamic-events.tsv"
        fi
        wait "$dynamic_pid" || true
        kill "$openbox_pid" 2>/dev/null || true
    else
        printf 'dynamic\tSKIP\txdotool-or-openbox-unavailable\n' >"$OUTPUT/dynamic-events.tsv"
    fi
fi

# Produce bounded evidence without host, certificate, user, frame, or clipboard data.
awk -F '\t' 'NR == 1 || NR > 1 { print }' "$OUTPUT/results.tsv"
printf 'results=%s\n' "$OUTPUT/results.tsv"
