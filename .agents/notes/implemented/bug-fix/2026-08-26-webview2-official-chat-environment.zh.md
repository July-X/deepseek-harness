# Agent Note：官方对话的 WebView2 同目录环境选项与诚实浏览器身份

Status: implemented

[English](2026-08-26-webview2-official-chat-environment.md) | 中文

## 问题

[Tauri 桌面外壳](../feature/2026-08-21-tauri-desktop-shell.zh.md)的 `official-chat` 窗口在 chat.deepseek.com 上以三种递进方式失败：默认配置触发站点的「使用环境异常」环境检查拦截页；第一轮加固（在共享配置目录上追加自定义浏览器参数）让窗口根本无法创建；随后的全套 Chrome 伪装仍在 HTTP 层声明与页面可观测事实之间留下可检测的矛盾。

## 决策

**一个 user-data 目录只承载一套环境选项。**WebView2 要求同一目录上创建的所有环境在全部选项上一致，附加浏览器参数也不例外。面板与工作台窗口已在默认目录上以默认选项建好环境，因此需要自定义参数的窗口必须钉住自己的 `.data_directory`；`open_official_chat` 使用 `<data_dir>/webview-official-chat`。选项不匹配时第二次环境创建直接失败，窗口永远不会出现。

**自定义浏览器参数会替换 wry 的默认值。**传入 `additional_browser_args` 会丢掉内置的 `--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection`，所以这些条目必须在 `OFFICIAL_CHAT_BROWSER_ARGS` 中重述，并叠加 `AutomationControlled`、`TranslateUI`、`InterestFeedContentSuggestions` 与 `--disable-blink-features=AutomationControlled`。仅 WebView2 后端消费该参数；macOS 与 Linux 忽略。

**窗口呈现真实的 Edge 身份，而不是伪装 Chrome。**user-agent 覆盖改不了 `Sec-CH-UA` 客户端提示与原生 `navigator.userAgentData`，头部声称 Chrome 而提示报告 Edge 正是环境检测盯防的跨层矛盾；普通 JS 对象与 `NavigatorUAData` 这类原生平台对象的可检测差异也无法弥合。builder 不再触碰 UA，在引擎层关闭自动化开关，并把 `chat-fingerprint.js` 收敛为两件事：把 `navigator.webdriver` 钉在 `false`（正常未自动化浏览器的值；`undefined` 本身就是 bot 特征），以及彻底删除 `__TAURI__` / `__TAURI_INTERNALS__` / `__TAURI_METADATA__` / `__TAURI_IPC__`。

## 已考虑的替代方案

**保留 Chrome 伪装并对齐其余信号。**否决：客户端提示无法从页面脚本或 user-agent 设置改写，不拦截响应就无法消除头层面的矛盾；而为遮盖一个破绽添加的每个 shim 都会引入新的与原生平台不一致的表面。

**共享默认目录但按窗口区分浏览器参数。**否决：同一目录上选项不匹配时环境创建失败——引入自定义参数后「打开按钮点了没反应」即此现象。

**继续把 `window.__TAURI__` 暴露为满足 `typeof` 检查的 neutered Proxy。**否决：任何 Tauri 全局的存在本身就是嵌入式信号；删除才能复现正常浏览器所呈现的事实。

## 后果

三个 webview 共存于同一进程，各窗口的选项集由目录隔离；专属配置目录在外壳数据目录下新增一个小目录。拉绳挂件不受影响，因为它在删除脚本运行之前已捕获 `window.__TAURI__`。如果 chat.deepseek.com 将来明确拒绝诚实的 Edge 身份，剩余手段是响应拦截以对齐品牌与客户端提示——刻意不予实现。
