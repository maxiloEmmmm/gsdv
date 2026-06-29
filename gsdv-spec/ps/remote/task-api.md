实现 gsdv 内置 remote HTTP API 与 agent WebSocket API。

## [x] 增加 remote server 设置项
新增 Web/API 服务运行设置。

已明确配置：

- 默认开启
- 默认端口 20050
- 默认监听 0.0.0.0
- 设置可以关闭
- 设置可以修改端口
- 设置可以修改监听地址

已确认：

- 设置项存储在 RuntimeSettings，随现有 `~/.gsdv/store` 保存
- UI 入口在 Settings -> Runtime 的 Remote server 小节
- 修改 remote server 设置后重启 listener

## [x] 设计 HTTP API 路由
定义 workspace 元数据与 agent 操作接口。

已明确需要接口：

- 获取 workspace 所有元数据
- 给某个 agent 发送文本输入
- 给某个 agent 发送 esc
- 给某个 agent 发送图

已确认路由：

- `GET /api/workspace` 获取 workspace 所有元数据
- `POST /api/agent/input` 给某个 agent 发送文本输入
- `POST /api/agent/esc` 给某个 agent 发送 esc
- `POST /api/agent/image` 给某个 agent 发送图

`GET /api/workspace` 成功响应：

```json
{
  "workspaces": [
    {
      "workspace_id": "string",
      "name": "string",
      "path": "string",
      "rows": [
        {
          "row_index": 0,
          "cols": [
            {
              "col_index": 0,
              "agents": [
                {
                  "agent_id": "string",
                  "title": "string"
                }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

`POST /api/agent/input` 请求：

```json
{
  "workspace_id": "string",
  "row_index": 0,
  "col_index": 0,
  "agent_id": "string",
  "text": "string"
}
```

`POST /api/agent/esc` 请求：

```json
{
  "workspace_id": "string",
  "row_index": 0,
  "col_index": 0,
  "agent_id": "string"
}
```

`POST /api/agent/image` 请求：

```json
{
  "workspace_id": "string",
  "row_index": 0,
  "col_index": 0,
  "agent_id": "string",
  "image_base64": "string",
  "mime_type": "image/png"
}
```

agent 操作成功响应：

```json
{
  "ok": true
}
```

错误响应：

```json
{
  "error": {
    "code": "agent_not_found",
    "message": "agent not found"
  }
}
```

错误码规范：

- `400`：请求路径或 JSON 参数错误
- `404`：agent 不存在，或对应 terminal host 未就绪
- `500`：内部错误

## [x] 设计 agent 输出 WebSocket 路由
定义绑定特定 agent 的 WebSocket 接口。

已明确行为：

- 连接绑定一个 agent
- 连接后发送该 agent 当前全部输出
- 后续输出更新持续发送

已确认路由：

- `GET /api/agent/output/ws?workspace_id=...&row_index=0&col_index=0&agent_id=...`

绑定参数：

- `workspace_id`：workspace 对外稳定标识
- `row_index`：agent 所在 row 数组下标
- `col_index`：agent 所在 col 数组下标
- `agent_id`：主 agent 或 subagent 的稳定 id

连接成功后，服务端先发送该 agent 当前 terminal buffer 的结构化 `snapshot`。

`snapshot` 消息：

```json
{
  "type": "snapshot",
  "sequence": 1,
  "cols": 120,
  "rows": [
    {
      "line_index": -12,
      "wrapped": false,
      "cells": [
        {
          "text": "a",
          "fg": "#111827",
          "bg": "#ffffff",
          "bold": false,
          "italic": false,
          "dim": false,
          "hidden": false,
          "inverse": false,
          "underline": "none",
          "strikeout": false,
          "wide": false
        }
      ]
    }
  ],
  "cursor": {
    "line_index": 23,
    "col_index": 4,
    "shape": "block"
  }
}
```

`append` 消息：

```json
{
  "type": "append",
  "sequence": 2,
  "rows": []
}
```

terminal 数据说明：

- `snapshot` 从 `alacritty_terminal` grid 导出当前 scrollback 和可见屏幕
- 当前 terminal scrollback 上限为 2000 行
- `fg` / `bg` 是按当前 gsdv theme 解析后的 `#RRGGBB`
- `underline` 可取 `none`、`single`、`double`、`curly`、`dotted`、`dashed`
- `wrapped` 来自 terminal cell 的 `WRAPLINE` 标记
- `append` 只推 scrollback 底部新增行
- 原地改写、spinner、全屏 TUI repaint、光标移动修改旧行不额外推 patch

心跳策略：

- 服务端每 30 秒发送一次 JSON `ping`
- 客户端收到后回复 JSON `pong`
- 服务端连续 2 次未收到 `pong` 后关闭连接

`ping` 消息：

```json
{
  "type": "ping",
  "sequence": 3
}
```

`pong` 消息：

```json
{
  "type": "pong",
  "sequence": 3
}
```

断线重连策略：

- 服务端不保留断线连接状态
- 客户端断线后重新连接同一个 URL
- 重连成功后服务端重新发送完整 `snapshot`，再继续发送 `append`

## [x] 接入唯一 AppEvent 队列
保证 remote server 不直接修改 UI 状态。

实现约束：

- HTTP/WS producer 只能投递 AppEvent
- AppEvent handler 保持轻量
- 慢操作必须异步完成后再投递结果事件
- agent 输入、esc、图片发送必须复用现有 terminal/agent owner 路径

## [ ] 接入 agent 输出广播源
让 WebSocket 能订阅 agent terminal 输出。

实现约束：

- 能读取连接时的当前 agent 输出
- 能在后续 terminal 更新时通知订阅者
- 不破坏现有 egui terminal repaint 逻辑
- 不让 WebSocket 订阅成为 UI 渲染路径的阻塞点
