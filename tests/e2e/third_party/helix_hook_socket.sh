#!/bin/sh
# 验证 Helix hook 方案依赖的外部机制：
# - app 级 Unix domain socket 能接收 4 字节 big-endian 长度帧。
# - payload 格式为 key:data，其中 key 可以是 helix.current。
# - 固定路径临时 config 能表达 Alt+d 到 :sh hook client 的映射。
# - app 每次启动 Helix 前重写同一个 config 文件，避免多名字堆积。
# 该脚本不驱动交互式 Helix TTY，只验证非代码机制的协议和配置形态。
set -eu

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/gsdv-helix-hook.XXXXXX")"
trap 'rm -rf "$tmp_dir"' EXIT

socket_path="$tmp_dir/hook.sock"
payload="helix.current:$tmp_dir/work/src/lib.rs:7"
config_path="$tmp_dir/helix-hook-config.toml"
received_path="$tmp_dir/received.txt"

python3 - "$socket_path" "$received_path" <<'PY' &
import socket
import struct
import sys

socket_path, received_path = sys.argv[1], sys.argv[2]
server = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
server.bind(socket_path)
server.listen(1)
conn, _ = server.accept()
header = conn.recv(4)
length = struct.unpack(">I", header)[0]
chunks = []
remaining = length
while remaining:
    chunk = conn.recv(remaining)
    if not chunk:
        raise SystemExit("socket closed before full payload")
    chunks.append(chunk)
    remaining -= len(chunk)
with open(received_path, "wb") as file:
    file.write(b"".join(chunks))
conn.close()
server.close()
PY
listener_pid=$!

while [ ! -S "$socket_path" ]; do
  sleep 0.05
done

python3 - "$socket_path" "$payload" <<'PY'
import socket
import struct
import sys

socket_path, payload = sys.argv[1], sys.argv[2].encode("utf-8")
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(socket_path)
client.sendall(struct.pack(">I", len(payload)) + payload)
client.close()
PY

wait "$listener_pid"

received="$(cat "$received_path")"
if [ "$received" != "$payload" ]; then
  echo "payload mismatch: $received" >&2
  exit 1
fi

gsdv_exe="$tmp_dir/current gsdv"
touch "$gsdv_exe"
keymap_command="A-d = \":sh '$gsdv_exe' hook-client --key helix.current --data %{buffer_name}:%{cursor_line}\""
cat > "$config_path" <<EOF
[keys.normal]
$keymap_command
EOF

if ! grep -Fq "$keymap_command" "$config_path"; then
  echo "helix keymap command was not written as expected" >&2
  exit 1
fi

if grep -q ':sh gsdv hook-client' "$config_path"; then
  echo "helix keymap must use the current executable path" >&2
  exit 1
fi

echo "ok"
