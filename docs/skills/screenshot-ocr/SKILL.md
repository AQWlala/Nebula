---
name: screenshot-ocr
version: 1.0.0
description: |
  截图 OCR 文字识别技能——对屏幕截图进行 OCR 提取文字，支持中英文混排。
  当用户要求"把这张截图里的字提取出来"、"识别一下屏幕上的文字"、"截个图做 OCR"时加载此技能。
  通过屏幕截取能力获取图像，再经 LLM 调用进行文字识别。对标 OpenAkita screenshot_ocr 能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "screen:capture"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# Screenshot OCR 技能（截图OCR文字识别）

## 概述

Screenshot OCR 是 Nebula 的屏幕文字识别技能，负责截取屏幕指定区域或全屏，
对图像中的文字进行 OCR 提取，支持中英文混排、表格结构、代码块等多种内容
形态。通过 `screen:capture` 能力获取截图，结合 `llm:call` 调用多模态大模型
或本地 OCR 引擎完成识别。

该技能解决"看到屏幕上的字但无法复制"的高频场景：图片型 PDF、视频字幕、
设计稿文案、远程桌面、错误弹窗等。识别结果保留原始排版结构，便于直接粘贴使用。

## 使用场景

- **图片文字提取**：把截图中的文字转为可编辑文本
- **视频字幕抓取**：暂停视频后截取画面，提取字幕文字
- **错误弹窗记录**：程序报错时截图 OCR，把错误信息转为文本贴进工单
- **设计稿文案**：从 Figma/PS 导出的设计图中提取文案用于校对
- **表格识别**：截图中的表格转为 Markdown/CSV 结构化数据
- **多语言识别**：中英日韩混排的截图一次性识别

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `region` | object | 否 | 截图区域 `{x, y, width, height}`，省略则全屏 |
| `image_path` | string | 否 | 直接指定已有图片路径，跳过截图步骤 |
| `languages` | string[] | 否 | 识别语言，默认 `["zh", "en"]`，可选 `ja`/`ko`/`fr` 等 |
| `output_format` | string | 否 | 输出格式：`text`（默认）/ `markdown` / `json` |
| `preserve_layout` | boolean | 否 | 是否保留原始排版，默认 `true` |
| `engine` | string | 否 | OCR 引擎：`auto`（默认）/ `local` / `cloud` |

示例输入：
```json
{
  "region": {"x": 100, "y": 200, "width": 800, "height": 400},
  "languages": ["zh", "en"],
  "output_format": "markdown",
  "preserve_layout": true
}
```

## 输出

```json
{
  "output": {
    "image_path": "C:/Users/nebula/.nebula/screenshots/2026-07-10-142301.png",
    "text": "# 错误报告\n\n运行时发生以下异常：\n- TypeError: Cannot read property 'name' of undefined\n- 位置：src/components/UserProfile.tsx:42",
    "blocks": [
      {"type": "heading", "text": "错误报告"},
      {"type": "paragraph", "text": "运行时发生以下异常："},
      {"type": "list", "items": ["TypeError: Cannot read property...", "位置：src/components/UserProfile.tsx:42"]}
    ],
    "languages_detected": ["zh", "en"],
    "confidence": 0.94
  },
  "error": null,
  "latency_ms": 2100
}
```

输出字段说明：
- `image_path`：截图保存路径，便于后续复用或调试
- `text`：识别出的纯文本或 Markdown
- `blocks`：结构化内容块（标题/段落/列表/表格等）
- `languages_detected`：检测到的语言列表
- `confidence`：整体识别置信度（0-1）

## 使用示例

### 示例 1：截取屏幕区域识别文字

用户："把屏幕左上角那段文字提取出来"

```json
{
  "region": {"x": 0, "y": 0, "width": 600, "height": 200},
  "output_format": "text"
}
```

技能截取指定区域并 OCR，返回纯文本结果。

### 示例 2：识别已有图片中的表格

用户："识别一下这张图里的表格"

```json
{
  "image_path": "D:/images/sales-table.png",
  "output_format": "markdown",
  "preserve_layout": true
}
```

技能识别表格结构并输出为 Markdown 表格，保留行列对应关系。

### 示例 3：全屏截图识别多语言

用户："截个全屏，把里面所有文字都识别出来"

```json
{
  "languages": ["zh", "en", "ja"],
  "output_format": "markdown"
}
```

截取全屏，自动检测中英日混排内容，按原始排版输出 Markdown。

## 注意事项

- **截图权限**：macOS 需要在"系统设置 > 隐私与安全 > 屏幕录制"中授权
  Nebula；Windows 与 Linux 通常无需额外授权。
- **图片尺寸**：建议截图分辨率不低于 72 DPI，过小图片（< 200px 宽度）
  识别准确率会下降。技能会自动放大低分辨率图片。
- **本地引擎**：`engine=local` 使用 PaddleOCR / Tesseract，离线运行，
  适合隐私敏感场景；`engine=cloud` 调用多模态 LLM，准确率更高但需联网。
- **隐私保护**：截图默认保存在用户目录 `.nebula/screenshots/`，30 天后
  自动清理。识别过程不持久化图片内容至云端（除非显式选择 cloud 引擎）。
- **手写文字**：当前版本不优选手写体识别，建议使用专门的 handwriting OCR。
- **依赖说明**：需要 Python 环境用于本地 OCR 引擎调用（PaddleOCR/Tesseract）。
  云端 OCR 由 Nebula 多模态 LLM 能力直接处理。
