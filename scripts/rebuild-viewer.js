const fs = require('fs');
const path = require('path');

const repoRoot = path.resolve(__dirname, '..');
const flowsDir = path.join(repoRoot, 'docs/superpowers/specs/2026-06-12-platform-flows-design');
const outFile = path.join(repoRoot, 'plans/diagrams-viewer-2026-06-12.html');

const filesConfig = [
  { name: 'surface-mac-cli.md', title: 'mac CLI' },
  { name: 'surface-win-cli.md', title: 'win CLI' },
  { name: 'surface-linux-cli.md', title: 'linux CLI' },
  { name: 'surface-mac-app.md', title: 'mac app (desktop)' },
  { name: 'surface-win-app.md', title: 'win app (desktop)' },
  { name: 'surface-android-app.md', title: 'android app' },
  { name: 'surface-ios-app.md', title: 'iOS app' },
  { name: 'interaction-capture-chain.md', title: '跨面: 手機→後端 擷取鏈交互' },
  { name: 'system-arch-deadspots.md', title: '跨面: 系統架構 + 死角' },
  { name: 'flows-synthesis.md', title: '跨面: 綜合流程彙整' }
];

let navLinks = '';
let sectionsHtml = '';
let totalDiagrams = 0;

for (const config of filesConfig) {
  const filePath = path.join(flowsDir, config.name);
  if (!fs.existsSync(filePath)) {
    console.error(`File not found: ${filePath}`);
    continue;
  }
  const content = fs.readFileSync(filePath, 'utf8');
  const id = config.name.replace('.md', '');
  navLinks += `<a href='#${id}'>${config.title}</a>`;

  const lines = content.split(/\r?\n/);
  let lastHeading = '';
  let inMermaid = false;
  let mermaidCode = [];
  let fileDiagrams = [];

  for (let line of lines) {
    const headingMatch = line.match(/^#{2,5}\s+(.*)/);
    if (headingMatch) {
      lastHeading = headingMatch[1].trim();
    }
    if (line.trim().startsWith('```mermaid')) {
      inMermaid = true;
      mermaidCode = [];
    } else if (line.trim().startsWith('```') && inMermaid) {
      inMermaid = false;
      const code = mermaidCode.join('\n');
      const isSeq = code.includes('sequenceDiagram');
      const typeLabel = isSeq ? '交互圖' : '流程圖';
      fileDiagrams.push({
        heading: lastHeading,
        type: typeLabel,
        code: code
      });
      totalDiagrams++;
    } else if (inMermaid) {
      mermaidCode.push(line);
    }
  }

  sectionsHtml += `<h2 id='${id}'>${config.title} <span class=c>— ${config.name} · ${fileDiagrams.length} 圖</span></h2>`;
  for (const diag of fileDiagrams) {
    let headingText = diag.heading.replace(/\[([^\]]+)\]\([^)]+\)/g, '$1');
    sectionsHtml += `<h3>${headingText} <span class=c>· ${diag.type}</span></h3>`;
    const escapedCode = diag.code
      .replace(/&/g, '&amp;')
      .replace(/</g, '&lt;')
      .replace(/>/g, '&gt;');
    sectionsHtml += `<pre class='mermaid'>${escapedCode}</pre>`;
  }
}

const html = `<!DOCTYPE html><html lang="zh-Hant"><head><meta charset="utf-8"><title>Phantom Mesh — 7-surface 流程圖 & 交互圖</title>
<script src="https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.min.js"></script>
<script>mermaid.initialize({startOnLoad:true,theme:'default',securityLevel:'loose'});</script>
<style>body{font-family:'Segoe UI','Microsoft JhengHei',sans-serif;max-width:1200px;margin:0 auto;padding:24px;color:#222}
h1{border-bottom:3px solid #333}h2{margin-top:2.2em;border-left:6px solid #2b6cb0;padding-left:10px;background:#eef4fb}
h3{color:#555;margin-top:1.4em}pre.mermaid{background:#fafafa;border:1px solid #e2e2e2;border-radius:8px;padding:14px;overflow:auto}
nav{position:sticky;top:0;background:#fff;border-bottom:1px solid #ddd;padding:8px 0;font-size:13px}nav a{margin-right:12px;color:#2b6cb0;text-decoration:none}.c{color:#999;font-weight:normal;font-size:13px}</style></head><body>
<h1>Phantom Mesh — 7-surface 流程圖 &amp; 交互圖 <span class=c>(mmdc 驗證 ${totalDiagrams}/${totalDiagrams} · zh-TW)</span></h1><nav>${navLinks}</nav>${sectionsHtml}</body></html>`;

fs.writeFileSync(outFile, html, 'utf8');
console.log(`Successfully generated ${outFile} with ${totalDiagrams} diagrams.`);
