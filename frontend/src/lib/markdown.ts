import { marked } from 'marked';
import DOMPurify from 'dompurify';

// Markdown → sanitized HTML.
//
// 助手内容来自 LLM,可被提示注入(读取恶意文件/网页时)诱导输出原始 HTML。
// marked.parse() 默认不消毒,因此必须经 DOMPurify 剥离 <script>、on* 事件处理器等,
// 再交给 {@html} 注入。叠加 tauri.conf.json 的严格 CSP 作为纵深防御。
//
// DOMPurify 默认配置保留 class 属性,故不影响 highlight.js 等基于 class 的代码高亮。
export function renderMarkdown(md: string): string {
  const raw = marked.parse(md || '', { async: false }) as string;
  return DOMPurify.sanitize(raw);
}
