在 web/ 下实现 Vue + AntV remote 页面。

## [x] 创建 web 工程
在代码目录 web/ 下创建前端工程。

已明确技术方向：

- Vue 3
- Vite
- pnpm
- ant-design-vue
- 单页面
- 需要随 gsdv 一起打包静态资源，构建产物后续由 Rust include 接入

待确认：

- Rust include 静态资源的具体接入方式

## [x] 实现页面布局
实现 remote 页面基础结构。

已明确布局：

- 页面顶部 header
- header 左侧有 left icon
- 点击 left icon 划出左侧抽屉
- 主区域展示 terminal 输出
- 底部输入框发送文本
## [x] 实现 workspace 抽屉 tree
在左侧抽屉展示 workspace 级别 tree。

树结构：

- workspace
- row
- col
- agent title

点击 agent title 后：

- 建立绑定该 agent 的 WebSocket
- 关闭抽屉
- 页面渲染为该 agent terminal 输出
## [x] 实现 agent terminal 输出视图
展示 WebSocket 推送的 agent 输出。

已明确行为：

- 初次连接显示当前 agent 全部输出
- 后续更新继续追加或刷新

已确认：

- 保留 ANSI 样式，并应用到 CSS 上
- 收到 snapshot/append 后自动滚动到底部
- 前端最多保留最近 2000 行
## [x] 实现 agent 输入框
底部输入框调用 API 给当前 agent 发送输入。

已确认：

- Enter 直接发送输入
- Shift+Enter 换行
- 输入框左边增加 ESC 按钮
- 图片入口为本地文件选择
## [x] 接入 web 构建与 remote 静态前端
增加 Makefile 构建入口，并让 remote HTTP 服务挂载前端构建产物。

已确认：

- remote server `/` 返回前端页面
- `/api/...` 继续作为接口路径
- 增加 `make build`
- `make build` 先构建 web 前端，再执行 `cargo build --release --locked --bin gsdv`
- GitHub Actions release 构建流程调用 Makefile，而不是直接手写 cargo build
- 增加 `make run`
- `make run` 使用 release 模式运行，即 `cargo run --release --bin gsdv`

实现注意：

- 需要参考现有 `.github/workflows/nightly-release.yml`
- 前端构建产物通过 Rust 二进制内嵌方式随应用一起可用
- 未构建 `web/dist/index.html` 时 Rust 编译失败
- 非 `/` 和 `/assets/...` 的前端路径返回 404
