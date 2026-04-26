#!/bin/sh

set -eu

OUTPUT_FILE="/tmp/test.output"

case "${1:-}" in
    metadata)
        input_value=''
        if [ -f "$OUTPUT_FILE" ]; then
            input_value=$(sed -n '1p' "$OUTPUT_FILE")
        fi
        printf '%s\n' "type: card"
        printf '%s\n' "action.0.key: touch"
        printf '%s\n' "input_need: true"
        printf '%s\n' "input_value: $input_value"
        printf '%s\n' "input_rows: 1"
        printf '%s\n' "refresh: 10"
        ;;
    value)
        # value 只输出文件内容；文件不存在时保持空输出。
        if [ -f "$OUTPUT_FILE" ]; then
            cat "$OUTPUT_FILE"
        fi
        ;;
    action)
        # 触发条件：Extra Tools 点击 touch action。
        # 不能直接无条件写入：同一个脚本协议可能扩展其他 action。
        # 防止回归：未知 action 误覆盖 /tmp/test.output。
        if [ "${_gsdv_action:-}" != "touch" ]; then
            printf '%s\n' "unknown action: ${_gsdv_action:-}" >&2
            exit 1
        fi
        printf '%s' "${__gsdv_input:-}" > "$OUTPUT_FILE"
        ;;
    *)
        printf '%s\n' "usage: $0 metadata|value|action" >&2
        exit 2
        ;;
esac
