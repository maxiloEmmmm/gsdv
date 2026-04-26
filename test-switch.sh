#!/bin/sh

set -eu

SWITCH_FILE="/tmp/test.switch"

case "${1:-}" in
    metadata)
        printf '%s\n' "type: switch"
        printf '%s\n' "input_need: false"
        printf '%s\n' "refresh: 10"
        ;;
    value)
        if [ -f "$SWITCH_FILE" ] && [ "$(sed -n '1p' "$SWITCH_FILE")" = "1" ]; then
            printf '%s\n' "true"
        else
            printf '%s\n' "false"
        fi
        ;;
    action)
        # 触发条件：Extra Tools switch 被用户切换。
        # 不能直接写入传入文本：存储协议要求 true=1、false=0。
        # 防止回归：UI 布尔值和 /tmp/test.switch 持久值语义错位。
        case "${_gsdv_action:-}" in
            true)
                printf '%s\n' "1" > "$SWITCH_FILE"
                ;;
            false)
                printf '%s\n' "0" > "$SWITCH_FILE"
                ;;
            *)
                printf '%s\n' "unknown switch value: ${_gsdv_action:-}" >&2
                exit 1
                ;;
        esac
        ;;
    *)
        printf '%s\n' "usage: $0 metadata|value|action" >&2
        exit 2
        ;;
esac
