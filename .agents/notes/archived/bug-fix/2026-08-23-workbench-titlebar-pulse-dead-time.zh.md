# Agent Note: 工作台标题栏脉冲关键帧 —— 把淡入淡出移到屏幕外，不缩短扫描距离

Status: implemented
Archived: 2026-08-28

[English](2026-08-23-workbench-titlebar-pulse-dead-time.md) | 中文

## 问题

桌面壳通过启动时向工作台 webview 注入一个 `<style>` 元素来接管 dsh web 工作台的 chrome（`desktop/src-tauri/src/titlebar-pulse.js`，按 `desktop/AGENTS.md` 中 chrome-row 脉冲的路径，通过 `WebviewWindowBuilder::initialization_script()` 加载——该脉冲同时安装给桌面壳自己的面板和内核的 web 工作台）。两道扫描原本使用如下关键帧：

```css
@keyframes dsh-workbench-titlebar-pulse-sweep {
  0%   { transform: translateX(-120%); opacity: 0; }
  15%  { opacity: 1; }
  85%  { opacity: 1; }
  100% { transform: translateX(360%); opacity: 0; }
}
```

动画为 `animation: ... 6.912s cubic-bezier(0.4, 0, 0.2, 1) infinite;`，第二道扫描以半周期 `animation-delay` 与第一道错相。

约 480px 宽、38% 带宽的 chrome 行使光带完全进入可见区域的位置大约是 `translateX(-100%)`，完全离开的位置大约是 `translateX(+162%)`。0% 关键帧把光带停在 `translateX(-120%)`（已经在左边缘外约 1/5 个带宽），100% 关键帧停在 `translateX(360%)`（远在右边缘外）。opacity 在前 15% 从 0 渐变到 1、在后 15% 从 1 渐变到 0，因此**有一部分渐变发生在光带位于 chrome 行内部时**；`cubic-bezier(0.4, 0, 0.2, 1)` 在两端减速——用户感知到的效果是「扫过一次，然后停顿数秒才再次扫描」。

本次修复按顺序尝试过三版：

1. **第一版**：把扫描弧线缩短到 `translateX(-100%) → 120%` 并收紧 opacity 渐变。用户审查时被否决：可见扫描变慢；`translateX(120%)` 把淡出落在光带后半段还在屏幕内时——光带「中途消失」。
2. **第二版**：保留原本的 480% 行程弧（`-120% → 360%`），把 opacity 渐变移到 8% / 85%，让淡入淡出完全发生在屏幕外。用户审查时再次被否决：`cubic-bezier(0.4, 0, 0.2, 1)` 在 t ≈ 0.30 处达到速度峰值，从那里开始单调减速到 t = 1.0。8% → 85% 这段（也就是光带在屏幕上的唯一行程段）会先加速、过峰值后再减速——光带在接近右边缘时明显慢下来并消失，读起来是「先加速、然后突然减速消失」。
3. **第三版（当前）**：关键帧与第二版相同，但把 `animation-timing-function` 从 `cubic-bezier(0.4, 0, 0.2, 1)` 换成 `linear`。光带现在以恒定速度穿越可见 chrome 行，opacity 渐变仍完全发生在屏幕外。

## 决策

两项改动一起：重写关键帧，让 opacity 渐变**完全发生在光带位于 chrome 行外时**；并把动画 timing function 从 `cubic-bezier(0.4, 0, 0.2, 1)` 换成 `linear`，让光带在可见行程段以恒定速度移动。

```css
@keyframes dsh-workbench-titlebar-pulse-sweep {
  0%   { transform: translateX(-120%); opacity: 0; }
  8%   { transform: translateX(-100%); opacity: 1; }
  85%  { transform: translateX(170%);  opacity: 1; }
  100% { transform: translateX(360%); opacity: 0; }
}

.row::after,
body > [data-titlebar-pulse='2'] {
  animation: dsh-workbench-titlebar-pulse-sweep 6.912s linear infinite;
}
```

在 6.912s 周期内的净效果（38% 带宽的光带穿越约 480px 宽的 chrome 行）：

- 0%（translateX -120%，opacity 0）→ 8%（translateX -100%，opacity 1）：光带停在左边缘外一个带宽处淡入，前端在 8% 这一刻正好越过 chrome 行的左边界，与 opacity 达到 1 同步。linear timing 让 opacity 在这 0.55s 内以恒定速率渐变。
- 8% → 85%（translateX 从 -100% 走到 170%，opacity 1）：光带完全不透明地穿过整个可见区域，到 85% 时尾端刚刚越过右边界。linear timing 给出恒定的移动速度——中段不加速，右边缘不减速。这 5.32s 是光带在屏幕上唯一的行程段。
- 85% → 100%（translateX 从 170% 走到 360%，opacity 1 → 0）：光带继续滑出屏幕到 translateX(360%)，同时 opacity 以恒定速率渐到 0。因为 85% 时光带已经离开 chrome 行，淡出不可见。

两道错相保留半周期 `animation-delay`（3.456s），让第二道在第一道还在穿越时进入屏幕。把淡出完全移到屏幕外且可见行程保持恒定速度后，「A 光带尾端离开行」到「B 光带头端进入行」之间的最坏间隔缩小到半周期减去光带的在屏行程时间——不到一秒，无可感知停顿，无可感知速度变化。

桌面壳面板自己的 `titlebar-pulse-sweep` 关键帧（`desktop/ui/styles.css`）保留原有形状——该面板以不同节拍（7.68s）渲染，不是用户报告的表面，改动它超出本次修复范围。两套关键帧有意保持差异；若桌面壳面板出现相同的死区症状，会在独立的改动中以同样方式处理。

`linear` 在这里合适的原因：光带的行程本身就是恒定速度的 `translateX` 扫描，淡入淡出段又短且在屏幕外；恒定速率的 opacity 渐变在视觉上读起来不像运动，只像一条线性扫描。`cubic-bezier(0.4, 0, 0.2, 1)`（Material "standard"）在 t ≈ 0.30 处达到速度峰值、从那里单调减速到 t = 1.0，会让光带在可见行程里呈现「先加速、过峰值、再减速到右边缘」的弧线——这正是第三版要去掉的症状。

## 备选方案

**把扫描弧线缩短到 `translateX(-100%) → 120%`，并收紧 opacity 渐变。** 不予采用：6.912s 周期不变，光带只走了原来不到一半的距离，可见扫描明显变慢。更糟的是 `translateX(120%)` 仅比右边缘多出一个带宽，光带后半段此时仍在屏幕内时 opacity 已经降到 0——光带「中途消失」。这是本修复的第一次尝试，用户在 commit 前几分钟就指出了错误。

**保留重写后的关键帧，但继续用 `cubic-bezier(0.4, 0, 0.2, 1)` 作为 timing function。** 不予采用：这条曲线在 t ≈ 0.30 处达到速度峰值，从那里单调减速到 t = 1.0。在屏行程完全位于 8% → 85%（t ≈ 0.08 到 t ≈ 0.85）这段，光带从 8% 起加速、过中段峰值、然后一直减速到右边缘——用户读起来是「先加速、然后突然减速消失」。这是本修复的第二次尝试，用户审查时再次被否决。

**保留同样的 `translateX` 端点，仅延长可见部分（例如 5% → 95% 替代 15% → 85%）。** 不予采用：起点和终点的 transform 仍然让光带在渐变期间有一部分位于行内——光带的前端或尾端在可见状态下淡入或淡出，感知到的死区时间不变，只是稍短。

**完全去掉淡入淡出（opacity 始终为 1，在 0% 和 100% 处硬切）。** 不予采用：光带自身的渐变已经在两端做了边缘渐隐；硬切的 opacity 会让前后两端 pop-in / pop-out 而不是平滑渐隐。保留 8% / 15% 的 opacity 渐变既保留了那道柔和的边缘，又把死区移到了屏幕之外。

**同步内核侧 `packages/client/web/src/base.css` 的标题栏脉冲，而不是由桌面壳覆盖。** 不予采用，因为 `desktop/AGENTS.md` 中工作台 UX 路径规定桌面壳拥有 chrome-row 样式权——让桌面壳覆盖能让工作台内容表面在跨内核升级时保持稳定，这正是 `titlebar-pulse.js` 文件带 `!important` 双保险的存在理由。

**把 CSS 动画换成 JS 驱动的 `requestAnimationFrame` 循环。** 不予采用，因为这会引入 Rust ↔ JS 注入面、破坏 HMR，并重复 `animation-iteration-count: infinite` 已经免费提供的效果。

**用 `ease-in-out` 或其他对称 easing 替代 `cubic-bezier(0.4, 0, 0.2, 1)`。** 不予采用，原因与第二条相同：任何在末端减速的曲线都会让光带在 opacity=1 期间「缓入」右边缘——正是第三版要去掉的视觉痕迹。`linear` 是标准 timing function 中唯一在整个周期里保持恒定速度的。

## 后果

注入脚本的 `css` 数组仍然只命名一个关键帧（`dsh-workbench-titlebar-pulse-sweep`）；第二道扫描（`body > [data-titlebar-pulse='2']`）通过 `animation-delay` 复用它。`titlebar-pulse.js` 中 `!important` 双保险的注释仍然准确：注入样式表始终赢得层叠，无论内核的 `base.css` 后续发布什么内容，整体规则块作为一个单元移动。

如果未来的内核版本加入自己的标题栏脉冲关键帧，桌面壳覆盖会继续通过 `body::before { content: none }` 和 `body::after` 规则抑制它们。本次修复不改变这一契约——只改变了覆盖内部的关键帧几何。

改动是单文件 `desktop/src-tauri/src/titlebar-pulse.js` 编辑。没有 Rust 代码路径变更，没有内核包被改动，桌面壳面板自己的脉冲保留当前节拍。验证为手工方式：通过桌面壳的 `tauri dev` 构建打开 `dsh web`，观察工作台 chrome 至少一个完整的 6.912s 周期，确认光带连续扫描，第一道退出与第二道进入之间没有可感知停顿，并确认可见扫描速度与原版一致（每周期一次整行穿越）。

本修复的第一次尝试已经落到 working tree 但在 commit 前被用户审查时发现错误；本 Note 记录的是修正后的关键帧和第一次尝试踩进的陷阱，避免未来的 agent 重复同样的错误。
