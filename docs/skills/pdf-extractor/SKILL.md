---
name: pdf-extractor
version: 1.0.0
description: |
  PDF 内容提取技能——提取 PDF 文档内容并结构化。当用户要求"提取 PDF 内容"、
  "把 PDF 转成文本"、"解析这个 PDF 的表格"时加载此技能。通过 file:read
  能力读取 PDF，再用 llm:call 能力识别结构（标题/段落/表格/图注），
  输出可机读的结构化数据。对标 Hermes PDF 解析能力。
author: Nebula Project
status: stable
capabilities: ["llm:call", "file:read"]
transport: local
dependencies: []
eligibility:
  bins: ["python"]
  env: []
  os: ["linux", "macos", "windows"]
min_nebula_version: "2.0.0"
---

# PDF Extractor 技能（PDF 内容提取）

## 概述

PDF Extractor 是 Nebula 的文档结构化技能，专门处理 PDF 这种"难啃"的格式。
PDF 本质是版面描述语言，同一份文档可能有文本层、图片层、表单层混合。
本技能负责把这些异构内容提取为统一的结构化表示，便于后续检索、摘要、
改写、翻译。

与 `file-reader` 的区别：`file-reader` 关注"读懂说了什么"（摘要），
`pdf-extractor` 关注"拆解成什么结构"（提取）。前者输出自然语言摘要，
后者输出结构化数据（JSON / Markdown / CSV）。

## 使用场景

- **文档数字化**：将扫描的 PDF 合同/报表转为可编辑文本
- **表格提取**：从 PDF 财报中提取资产负债表为 Excel/CSV
- **学术论文**：提取论文的标题、作者、摘要、参考文献结构
- **批量入库**：把 PDF 档案提取为 Markdown 存入知识库
- **表单填充**：提取 PDF 表单字段名与值，用于自动化填充
- **图文分离**：提取 PDF 正文文本，剥离图片与页眉页脚

## 输入

| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `path` | string | 是 | PDF 文件绝对路径 |
| `output_format` | string | 否 | 输出格式：`markdown` / `json` / `csv` / `txt`（默认 markdown） |
| `pages` | string | 否 | 页码范围，如 "1-5" / "3,7,9" / "all"（默认） |
| `extract_tables` | boolean | 否 | 是否专门提取表格，默认 true |
| `extract_images` | boolean | 否 | 是否提取图片元信息（不导出图片本体），默认 false |
| `ocr_fallback` | boolean | 否 | 无文本层时是否启用 OCR，默认 false |

示例输入：
```json
{
  "path": "D:/docs/financial-report-2026.pdf",
  "output_format": "json",
  "pages": "1-10",
  "extract_tables": true
}
```

## 输出

```json
{
  "output": {
    "file": "financial-report-2026.pdf",
    "total_pages": 42,
    "extracted_pages": 10,
    "format": "json",
    "content": {
      "title": "2026 年度财务报告",
      "sections": [
        {
          "heading": "一、公司概况",
          "level": 1,
          "page": 1,
          "text": "本公司成立于..."
        },
        {
          "heading": "二、资产负债表",
          "level": 1,
          "page": 5,
          "tables": [
            {
              "caption": "表 1：合并资产负债表",
              "headers": ["项目", "期末余额", "期初余额"],
              "rows": [
                ["货币资金", "1,200,000", "980,000"],
                ["应收账款", "560,000", "620,000"]
              ]
            }
          ]
        }
      ]
    },
    "tables_count": 8,
    "chars_extracted": 24500
  },
  "error": null,
  "latency_ms": 7800
}
```

输出字段说明：
- `content.sections`：按标题层级拆分的文档结构树
- `content.sections[].tables`：该章节下的表格（含表头与行数据）
- `tables_count`：全文提取的表格总数
- `chars_extracted`：提取的字符总数

## 使用示例

### 示例 1：PDF 转 Markdown

用户："把这个 PDF 转成 Markdown"

```json
{
  "path": "D:/papers/ai-agent-survey-2026.pdf",
  "output_format": "markdown",
  "pages": "all"
}
```

输出保留标题层级的 Markdown 文档，表格转为 MD 表格语法。

### 示例 2：提取财务报表表格

用户："把这份财报里的资产负债表提取成 Excel 能用的格式"

```json
{
  "path": "D:/docs/financial-report-2026.pdf",
  "output_format": "csv",
  "pages": "5-8",
  "extract_tables": true
}
```

输出 CSV 文件，每个表格独立一段，可直接导入 Excel。

### 示例 3：扫描件 OCR 提取

用户："这是扫描的合同，帮我提取文字"

```json
{
  "path": "D:/contracts/scanned-contract.pdf",
  "output_format": "txt",
  "ocr_fallback": true
}
```

检测到无文本层后自动启用 OCR，输出纯文本。注意 OCR 耗时较长。

## 注意事项

- **格式复杂性**：PDF 是版面描述格式，非结构化文档。复杂排版（多栏、
  脚注、浮动文本框）可能导致提取顺序错乱，需人工校对。
- **扫描件处理**：纯图片型 PDF（扫描件）无文本层，需启用 `ocr_fallback`。
  OCR 依赖 Python 的 pytesseract 或 PaddleOCR，首次使用需安装模型。
- **表格识别**：表格提取基于线条检测 + 单元格对齐分析。无边框表格
  （仅靠空格对齐）识别率较低，可能误判为普通文本。
- **加密 PDF**：密码保护的 PDF 需用户先提供密码，技能不尝试破解。
  读取加密 PDF 时会返回 `"error": "encrypted, password required"`。
- **大文件限制**：超过 100 页的 PDF 分批提取（每批 20 页），避免内存
  溢出。超过 500 页时提示用户指定 `pages` 范围。
- **图片处理**：`extract_images` 仅返回图片元信息（位置、尺寸、格式），
  不导出图片本体文件。如需导出图片，请使用专门的 PDF 工具。
- **隐私保护**：PDF 内容仅在本地处理，不上传外部服务。OCR 模型本地
  运行，不依赖云 OCR API。
- **依赖说明**：需要 Python 环境与 pdfplumber / PyMuPDF 库。OCR 功能
  额外依赖 pytesseract 或 PaddleOCR。
