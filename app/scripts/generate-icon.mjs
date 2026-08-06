/**
 * Generate Spectyn Mesh icon — Dark mechanical mask v2
 * Sharp angular mask, tall pointed horns, bright cyan eyes, visible mesh/circuit below
 */
import sharp from 'sharp';

const SIZE = 1024;

function generateSpectynMaskSVG() {
  let seed = 77;
  function rand() {
    seed = (seed * 1664525 + 1013904223) & 0x7fffffff;
    return seed / 0x7fffffff;
  }

  // Generate mesh wiring
  const meshNodes = [];
  const meshLines = [];
  const tendrils = [];

  // Dense mesh below mask
  for (let i = 0; i < 200; i++) {
    const x = 180 + rand() * 664;
    const y = 560 + rand() * 440;
    const cx = Math.abs(x - 512) / 332;
    if (rand() < (1 - cx * 0.5)) {
      meshNodes.push({ x, y, size: 1 + rand() * 3, bright: rand() > 0.75 });
    }
  }

  // Also add nodes on/around mask body
  for (let i = 0; i < 60; i++) {
    const x = 280 + rand() * 464;
    const y = 250 + rand() * 350;
    meshNodes.push({ x, y, size: 0.8 + rand() * 1.5, bright: rand() > 0.85 });
  }

  for (let i = 0; i < meshNodes.length; i++) {
    const dists = [];
    for (let j = i + 1; j < meshNodes.length; j++) {
      const dx = meshNodes[i].x - meshNodes[j].x;
      const dy = meshNodes[i].y - meshNodes[j].y;
      const d = Math.sqrt(dx * dx + dy * dy);
      if (d < 70) dists.push({ j, d });
    }
    dists.sort((a, b) => a.d - b.d);
    for (let k = 0; k < Math.min(3, dists.length); k++) {
      meshLines.push({
        x1: meshNodes[i].x, y1: meshNodes[i].y,
        x2: meshNodes[dists[k].j].x, y2: meshNodes[dists[k].j].y,
        opacity: 0.06 + (1 - dists[k].d / 70) * 0.2,
      });
    }
  }

  // Hanging wire tendrils
  for (let i = 0; i < 25; i++) {
    const startX = 310 + i * 17 + (rand() - 0.5) * 12;
    const startY = 555 + rand() * 30;
    const endX = startX + (rand() - 0.5) * 100;
    const endY = startY + 120 + rand() * 350;
    const cp1x = startX + (rand() - 0.5) * 70;
    const cp1y = startY + 40 + rand() * 80;
    const cp2x = endX + (rand() - 0.5) * 50;
    const cp2y = endY - 40 - rand() * 80;
    tendrils.push({ startX, startY, cp1x, cp1y, cp2x, cp2y, endX, endY, w: 0.4 + rand() * 2 });
  }

  let svg = `<svg xmlns="http://www.w3.org/2000/svg" width="${SIZE}" height="${SIZE}" viewBox="0 0 ${SIZE} ${SIZE}">
  <defs>
    <radialGradient id="bg" cx="50%" cy="35%" r="65%">
      <stop offset="0%" stop-color="#0c0c18"/>
      <stop offset="60%" stop-color="#060610"/>
      <stop offset="100%" stop-color="#020206"/>
    </radialGradient>
    <linearGradient id="metal1" x1="30%" y1="0%" x2="70%" y2="100%">
      <stop offset="0%" stop-color="#3a3a52"/>
      <stop offset="20%" stop-color="#55556e"/>
      <stop offset="40%" stop-color="#3e3e56"/>
      <stop offset="60%" stop-color="#4a4a64"/>
      <stop offset="80%" stop-color="#353550"/>
      <stop offset="100%" stop-color="#2e2e46"/>
    </linearGradient>
    <linearGradient id="metal2" x1="0%" y1="0%" x2="100%" y2="80%">
      <stop offset="0%" stop-color="#50506a"/>
      <stop offset="50%" stop-color="#3a3a54"/>
      <stop offset="100%" stop-color="#28283e"/>
    </linearGradient>
    <linearGradient id="hornL" x1="100%" y1="100%" x2="0%" y2="0%">
      <stop offset="0%" stop-color="#3a3a52"/>
      <stop offset="50%" stop-color="#5a5a72"/>
      <stop offset="100%" stop-color="#2a2a42"/>
    </linearGradient>
    <linearGradient id="hornR" x1="0%" y1="100%" x2="100%" y2="0%">
      <stop offset="0%" stop-color="#3a3a52"/>
      <stop offset="50%" stop-color="#5a5a72"/>
      <stop offset="100%" stop-color="#2a2a42"/>
    </linearGradient>
    <linearGradient id="wireG" x1="50%" y1="0%" x2="50%" y2="100%">
      <stop offset="0%" stop-color="#5a5a78"/>
      <stop offset="40%" stop-color="#3a3a56"/>
      <stop offset="100%" stop-color="#1a1a2e" stop-opacity="0.2"/>
    </linearGradient>
    <radialGradient id="eyeG" cx="50%" cy="50%" r="50%">
      <stop offset="0%" stop-color="white" stop-opacity="1"/>
      <stop offset="15%" stop-color="#aaffff" stop-opacity="0.95"/>
      <stop offset="35%" stop-color="#06f0ff" stop-opacity="0.8"/>
      <stop offset="60%" stop-color="#06b6d4" stop-opacity="0.4"/>
      <stop offset="100%" stop-color="#06b6d4" stop-opacity="0"/>
    </radialGradient>
    <filter id="glow1" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="4"/></filter>
    <filter id="eyeF" x="-100%" y="-100%" width="300%" height="300%">
      <feGaussianBlur stdDeviation="12" result="b"/>
      <feMerge><feMergeNode in="b"/><feMergeNode in="b"/><feMergeNode in="b"/><feMergeNode in="SourceGraphic"/></feMerge>
    </filter>
    <filter id="shadowF">
      <feDropShadow dx="0" dy="6" stdDeviation="15" flood-color="#000" flood-opacity="0.7"/>
    </filter>
    <clipPath id="rr"><rect width="1024" height="1024" rx="190" ry="190"/></clipPath>
  </defs>

  <g clip-path="url(#rr)">
    <rect width="${SIZE}" height="${SIZE}" fill="url(#bg)"/>

    <!-- BG circuit traces -->
    <g opacity="0.035" stroke="#4a4a6e" stroke-width="0.6" fill="none">`;

  for (let i = 0; i < 40; i++) {
    const x = rand() * SIZE;
    const y = rand() * SIZE;
    const segs = 2 + Math.floor(rand() * 3);
    let d = `M ${x.toFixed(0)} ${y.toFixed(0)}`;
    let cx = x, cy = y;
    for (let s = 0; s < segs; s++) {
      if (rand() > 0.5) { cx += (rand() - 0.3) * 100; }
      else { cy += (rand() - 0.3) * 100; }
      d += ` L ${cx.toFixed(0)} ${cy.toFixed(0)}`;
    }
    svg += `\n      <path d="${d}"/>`;
    svg += `\n      <circle cx="${cx.toFixed(0)}" cy="${cy.toFixed(0)}" r="2.5" fill="#4a4a6e"/>`;
  }

  svg += `
    </g>

    <!-- Mesh lines -->
    <g stroke="#3a3a60" fill="none">`;
  for (const l of meshLines) {
    svg += `\n      <line x1="${l.x1.toFixed(0)}" y1="${l.y1.toFixed(0)}" x2="${l.x2.toFixed(0)}" y2="${l.y2.toFixed(0)}" stroke-width="0.7" opacity="${l.opacity.toFixed(2)}"/>`;
  }
  svg += `</g>

    <!-- Tendrils -->
    <g fill="none" stroke="url(#wireG)" stroke-linecap="round">`;
  for (const t of tendrils) {
    svg += `\n      <path d="M${t.startX.toFixed(0)} ${t.startY.toFixed(0)} C${t.cp1x.toFixed(0)} ${t.cp1y.toFixed(0)},${t.cp2x.toFixed(0)} ${t.cp2y.toFixed(0)},${t.endX.toFixed(0)} ${t.endY.toFixed(0)}" stroke-width="${t.w.toFixed(1)}" opacity="${(0.15 + rand() * 0.3).toFixed(2)}"/>`;
  }
  svg += `</g>

    <!-- Mesh nodes -->
    <g>`;
  for (const n of meshNodes) {
    const c = n.bright ? '#06d4d4' : '#4a4a6e';
    const op = n.bright ? 0.5 + rand() * 0.3 : 0.08 + rand() * 0.15;
    svg += `\n      <circle cx="${n.x.toFixed(0)}" cy="${n.y.toFixed(0)}" r="${n.size.toFixed(1)}" fill="${c}" opacity="${op.toFixed(2)}"/>`;
  }
  svg += `</g>

    <!-- ═══ MASK ═══ -->
    <g filter="url(#shadowF)">

      <!-- Mask body - wider, more angular -->
      <path d="
        M 512 195
        L 395 215  L 330 245  L 295 310  L 285 380  L 300 440
        L 330 490  L 370 525  L 420 548  L 470 560  L 512 565
        L 554 560  L 604 548  L 654 525  L 684 490  L 714 440
        L 729 380  L 719 310  L 684 245  L 619 215  L 512 195 Z
      " fill="url(#metal1)" stroke="#1a1a30" stroke-width="1.5"/>

      <!-- Left horn — taller, sharper -->
      <path d="M 395 215 L 260 60 L 295 310 L 330 245 Z" fill="url(#hornL)" stroke="#1a1a30" stroke-width="1"/>
      <path d="M 370 210 L 268 72 L 295 280" fill="none" stroke="#6a6a8a" stroke-width="0.6" opacity="0.35"/>
      <!-- Horn edge highlight -->
      <path d="M 395 215 L 260 60" stroke="#7a7a9a" stroke-width="0.8" opacity="0.25" fill="none"/>

      <!-- Right horn — taller, sharper -->
      <path d="M 619 215 L 764 60 L 719 310 L 684 245 Z" fill="url(#hornR)" stroke="#1a1a30" stroke-width="1"/>
      <path d="M 644 210 L 756 72 L 719 280" fill="none" stroke="#6a6a8a" stroke-width="0.6" opacity="0.35"/>
      <path d="M 619 215 L 764 60" stroke="#7a7a9a" stroke-width="0.8" opacity="0.25" fill="none"/>

      <!-- Forehead crest -->
      <path d="M 430 220 L 512 200 L 594 220 L 575 260 L 512 250 L 449 260 Z"
            fill="#44445e" opacity="0.5" stroke="#2a2a40" stroke-width="0.5"/>

      <!-- Center ridge -->
      <path d="M 505 250 L 512 230 L 519 250 L 516 410 L 512 430 L 508 410 Z"
            fill="#4e4e68" opacity="0.45"/>

      <!-- Cheek armor L -->
      <path d="M 295 330 L 360 300 L 385 370 L 360 440 L 305 420 Z"
            fill="#383852" opacity="0.35" stroke="#26263e" stroke-width="0.4"/>
      <!-- Cheek armor R -->
      <path d="M 719 330 L 654 300 L 629 370 L 654 440 L 709 420 Z"
            fill="#383852" opacity="0.35" stroke="#26263e" stroke-width="0.4"/>

      <!-- Jaw plates -->
      <path d="M 370 500 L 420 520 L 470 535 L 512 540 L 554 535 L 604 520 L 654 500 L 640 530 L 580 555 L 512 562 L 444 555 L 384 530 Z"
            fill="#323250" opacity="0.4"/>

      <!-- Surface seams -->
      <g stroke="#22223a" stroke-width="0.7" fill="none" opacity="0.6">
        <line x1="360" y1="290" x2="430" y2="270"/>
        <line x1="654" y1="290" x2="584" y2="270"/>
        <line x1="340" y1="460" x2="420" y2="500"/>
        <line x1="674" y1="460" x2="594" y2="500"/>
        <line x1="390" y1="390" x2="450" y2="410"/>
        <line x1="624" y1="390" x2="564" y2="410"/>
        <line x1="445" y1="225" x2="432" y2="340"/>
        <line x1="569" y1="225" x2="582" y2="340"/>
        <line x1="350" y1="350" x2="310" y2="400"/>
        <line x1="664" y1="350" x2="704" y2="400"/>
      </g>

      <!-- Rivets -->
      <g fill="#5e5e7e" opacity="0.45">
        <circle cx="360" cy="265" r="3.5"/><circle cx="654" cy="265" r="3.5"/>
        <circle cx="320" cy="390" r="3"/><circle cx="694" cy="390" r="3"/>
        <circle cx="420" cy="220" r="2.5"/><circle cx="594" cy="220" r="2.5"/>
        <circle cx="455" cy="535" r="2.5"/><circle cx="569" cy="535" r="2.5"/>
        <circle cx="380" cy="470" r="2"/><circle cx="634" cy="470" r="2"/>
        <circle cx="300" cy="340" r="2"/><circle cx="714" cy="340" r="2"/>
      </g>

      <!-- Metallic edge highlights -->
      <path d="M 512 195 L 395 215 L 260 60" fill="none" stroke="#8888aa" stroke-width="0.7" opacity="0.2"/>
      <path d="M 512 195 L 619 215 L 764 60" fill="none" stroke="#8888aa" stroke-width="0.7" opacity="0.2"/>
      <path d="M 285 380 L 300 440 L 370 525" fill="none" stroke="#6666880" stroke-width="0.5" opacity="0.15"/>
      <path d="M 729 380 L 714 440 L 654 525" fill="none" stroke="#666688" stroke-width="0.5" opacity="0.15"/>
    </g>

    <!-- ═══ EYES ═══ -->
    <!-- Left eye -->
    <g filter="url(#eyeF)">
      <path d="M 340 330 L 410 290 L 475 315 L 455 355 L 385 365 L 340 350 Z"
            fill="#020208"/>
      <path d="M 352 333 L 408 298 L 465 319 L 448 350 L 388 358 L 350 345 Z"
            fill="#06f0ff" opacity="0.12"/>
      <path d="M 370 330 L 412 306 L 455 322 L 442 345 L 392 352 L 365 342 Z"
            fill="#06f0ff" opacity="0.3"/>
      <path d="M 388 326 L 418 312 L 445 325 L 436 340 L 400 344 L 385 336 Z"
            fill="#06f0ff" opacity="0.55"/>
      <ellipse cx="413" cy="328" rx="14" ry="7" fill="#aaffff" opacity="0.85"/>
      <ellipse cx="413" cy="328" rx="7" ry="4" fill="white" opacity="0.95"/>
    </g>

    <!-- Right eye -->
    <g filter="url(#eyeF)">
      <path d="M 684 330 L 614 290 L 549 315 L 569 355 L 639 365 L 684 350 Z"
            fill="#020208"/>
      <path d="M 672 333 L 616 298 L 559 319 L 576 350 L 636 358 L 674 345 Z"
            fill="#06f0ff" opacity="0.12"/>
      <path d="M 654 330 L 612 306 L 569 322 L 582 345 L 632 352 L 659 342 Z"
            fill="#06f0ff" opacity="0.3"/>
      <path d="M 636 326 L 606 312 L 579 325 L 588 340 L 624 344 L 639 336 Z"
            fill="#06f0ff" opacity="0.55"/>
      <ellipse cx="611" cy="328" rx="14" ry="7" fill="#aaffff" opacity="0.85"/>
      <ellipse cx="611" cy="328" rx="7" ry="4" fill="white" opacity="0.95"/>
    </g>

    <!-- Eye light cast -->
    <ellipse cx="413" cy="370" rx="55" ry="25" fill="#06b6d4" opacity="0.05"/>
    <ellipse cx="611" cy="370" rx="55" ry="25" fill="#06b6d4" opacity="0.05"/>

    <!-- Vignette -->
    <radialGradient id="vig" cx="50%" cy="38%" r="60%">
      <stop offset="0%" stop-color="transparent"/>
      <stop offset="80%" stop-color="transparent"/>
      <stop offset="100%" stop-color="#020206" stop-opacity="0.6"/>
    </radialGradient>
    <rect width="${SIZE}" height="${SIZE}" fill="url(#vig)"/>
    <rect width="1024" height="1024" rx="190" ry="190" fill="none" stroke="#1a1a2e" stroke-width="2" opacity="0.4"/>
  </g>
</svg>`;
  return svg;
}

const svg = generateSpectynMaskSVG();

async function main() {
  const iconDir = new URL('../src-tauri/icons/', import.meta.url).pathname.replace(/^\/([A-Z]:)/, '$1');
  const masterPng = await sharp(Buffer.from(svg))
    .resize(1024, 1024)
    .png({ quality: 100, compressionLevel: 9 })
    .toBuffer();
  const { writeFile } = await import('fs/promises');
  await writeFile(`${iconDir}/icon-source.png`, masterPng);
  await writeFile(`${iconDir}/app-icon.png`, masterPng);
  await writeFile(`${iconDir}/icon.png`, masterPng);
  console.log('✅ Spectyn Mesh mask icon v2 generated');
}

main().catch(e => { console.error(e); process.exit(1); });
