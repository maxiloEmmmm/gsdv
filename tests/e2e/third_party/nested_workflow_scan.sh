#!/usr/bin/env bash
# 验证嵌套 workflow 扫描的文件系统边界规则。
#
# 场景：workspace 自身可以有 .git；扫描会继续进入普通目录，命中子目录
# 的 .git 文件或目录后只检查该 repo 自身的 gsdv-spec/root.md，并停止深入。
# 该脚本只使用临时目录，不修改真实 workspace。
# 示例：./tests/e2e/third_party/nested_workflow_scan.sh -> ok

set -euo pipefail

# 收集含 workflow 的嵌套 Git repo，输出 workspace 相对路径。
# 示例：scan "$tmp/workspace" -> services/api
scan() {
    local workspace="$1"
    local relative_dir="${2:-}"
    local absolute_dir="$workspace${relative_dir:+/$relative_dir}"
    local entry name child git_marker

    for entry in "$absolute_dir"/* "$absolute_dir"/.[!.]* "$absolute_dir"/..?*; do
        [[ -d "$entry" && ! -L "$entry" ]] || continue
        name="${entry##*/}"
        [[ "$name" != .git ]] || continue
        child="${relative_dir:+$relative_dir/}$name"
        git_marker="$entry/.git"
        if [[ -e "$git_marker" || -L "$git_marker" ]]; then
            [[ -f "$entry/gsdv-spec/root.md" ]] && printf '%s\n' "$child"
        else
            scan "$workspace" "$child"
        fi
    done
}

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-nested-workflow.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT
workspace="$tmp_dir/workspace"
mkdir -p "$workspace/.git" "$workspace/services/api/.git/objects" \
    "$workspace/services/api/gsdv-spec" \
    "$workspace/services/api/vendor/ignored/gsdv-spec" \
    "$workspace/tools/plain/nested/.git" "$workspace/tools/plain/nested/gsdv-spec" \
    "$workspace/no-spec/.git/objects" "$workspace/no-spec/deeper/.git"
touch "$workspace/services/api/gsdv-spec/root.md" \
    "$workspace/services/api/vendor/ignored/gsdv-spec/root.md" \
    "$workspace/tools/plain/nested/gsdv-spec/root.md"

mapfile -t discovered < <(scan "$workspace")
[[ "${#discovered[@]}" -eq 2 ]] || {
    printf 'expected 2 nested workflows, found %s\n' "${#discovered[@]}" >&2
    exit 1
}
[[ "${discovered[0]}" == "services/api" ]] || exit 1
[[ "${discovered[1]}" == "tools/plain/nested" ]] || exit 1
if printf '%s\n' "${discovered[@]}" | grep -Fxq 'services/api/vendor/ignored'; then
    printf 'scanner crossed a git boundary\n' >&2
    exit 1
fi

printf 'ok: nested workflow scan boundaries\n'
