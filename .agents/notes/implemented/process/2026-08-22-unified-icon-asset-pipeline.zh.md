# Agent Note: Unified icon asset pipeline

Status: implemented

[English](2026-08-22-unified-icon-asset-pipeline.md) | 中文

## Problem

鲸鱼图标此前是各处手工复制的文件，彼此已经漂移：`desktop/src-tauri/icons/` 由单一 512px PNG 经 `tauri icon` 生成，`website/public/favicon.svg` 是没有红眼的品牌蓝鲸鱼，`apps/web/public/favicon.svg` 是没有红眼的深色模式自适应鲸鱼，`desktop/ui/whale-icon.png` 是手工渲染的。红眼重设计让这种漂移暴露出来；而且完整眼部细节（50 单位 viewBox 里 r=0.62 的光晕、星芒、射线）在 16–64px 下是亚像素，单一母版在物理上无法服务所有尺寸。

## Decision

`desktop/assets/` 下两个 SVG 母版是唯一手工维护的图标源：`whale-icon.svg`（完整红眼细节，用于 ≥128px）与 `whale-icon-small.svg`（红眼夸大版——光晕与眼核更大、射线更粗、用大高光点取代亚像素星芒——用于 ≤64px 与 favicon 投影）。`desktop/scripts/build-icons.sh` 一次运行再生成全部产物：`src-tauri/icons/`（逐尺寸渲染相应母版后由 ImageMagick 合成 `icon.ico`、iconset + `iconutil` 合成 `icon.icns`，而非从单一位图降采样）、`assets/whale-icon-512.png`、`ui/whale-icon.png`（用小型母版——面板侧栏只按 60 CSS px 显示，完整眼睛会是亚像素），以及由小母版文本投影生成的两个 SVG favicon——`website` 用品牌蓝（`#4D6BFE`）鲸身，`apps/web` 用 `fill="#000"` 并带 `prefers-color-scheme: dark` 样式块（由 `apps/web/tests/pwa-manifest.e2e.ts` 锁定）。

两个母版中眼睛射线一律用 `<polygon>` 而非 `<path>`：apps/web 的深色模式规则选择器是 `path`，若射线用 path 绘制会随鲸身一起被漂白成白色。

`tauri icon` 不再属于该流程：它从单一母版再生成所有帧，会悄悄把小尺寸帧退回完整细节版。

## Alternatives considered

**单母版 + 全部走 `tauri icon`。** 即原流程，工具最少。但它无法表达随尺寸变化的设计：16–64px 下完整红眼不可见，品牌特征恰好在图标最常见的尺寸消失。

**各使用处手工同步。** 每个场景各管各的文件，由人手工对齐。这正是本次要修的漂移成因：三个文件、三种剪影、两种眼睛处理，没有任何流程把它们连起来。

**再设一个 favicon 专用母版。** favicon 就是小图标，小尺寸母版已经带有它需要的简化。单独立一个文件只会重新打开这条流水线要关闭的漂移口。

## Consequences

一次设计改动最多触及两个 SVG 加一次脚本运行，仓库内所有 png/ico/icns/favicon 随之更新。脚本依赖 rsvg-convert、ImageMagick 和 macOS `iconutil`，图标再生成是 macOS 步骤，CI 不消费它。`website/public/wordmark.svg` 不受影响——它不含鲸鱼图形。`apps/web` favicon 保留深色模式行为，website favicon 保留品牌蓝鲸身：统一的是剪影与红眼设计，而不是各媒介的配色处理。
