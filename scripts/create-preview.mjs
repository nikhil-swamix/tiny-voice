import {readFile} from 'node:fs/promises';
import {chromium} from 'playwright-core';

const sample = 'data:image/png;base64,' + (await readFile('widget-output-mini.png')).toString('base64');
const browser = await chromium.launch({executablePath: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe', headless: true, args: ['--disable-gpu']});
try {
  const page = await browser.newPage({viewport: {width: 1600, height: 1000}, deviceScaleFactor: 1});
  await page.setContent(`<html><head><style>
    *{box-sizing:border-box}body{margin:0;background:#000;color:white;font-family:Segoe UI,system-ui}
    main{width:1600px;height:1000px;padding:76px 96px;position:relative;overflow:hidden;border:1px solid #242424}
    header{font-size:17px;font-weight:600;letter-spacing:4px}header span{float:right;letter-spacing:1px;font-size:12px;color:#aaa;font-weight:400}
    h1{font-size:92px;line-height:1.08;letter-spacing:-5px;margin:94px 0 28px;font-weight:650}
    .lead{color:#bdbdbd;font-size:25px;line-height:1.5;max-width:560px;margin:0}
    .features{display:flex;gap:38px;margin-top:78px}section{border-top:1px solid #555;padding-top:20px}section b{font-size:17px}section p{font-size:14px;line-height:1.5;color:#aaa;margin:8px 0}
    figure{position:absolute;right:152px;top:160px;margin:0;width:360px}figure img{width:360px;display:block;image-rendering:auto}figcaption{font-size:12px;color:#999;margin-top:20px;text-align:center;letter-spacing:1px}
    footer{position:absolute;bottom:67px;left:96px;right:96px;border-top:1px solid #333;padding-top:22px;color:#bdbdbd;font-size:14px}footer span{float:right}
  </style></head><body><main><header>TINY VOICE<span>VOICE TO TASK NOTES</span></header>
    <h1>Speak.<br>Structure.<br>Paste.</h1><p class="lead">Clear task notes from your voice.<br>A tiny widget that stays with your workflow.</p>
    <div class="features"><section><b>Live preview</b><p>Your words,<br>as you speak.</p></section><section><b>Clean structure</b><p>Tasks, stages<br>and useful hints.</p></section><section><b>Automatic paste</b><p>Finished notes,<br>right where you work.</p></section></div>
    <figure><img src="${sample}" alt="Tiny Voice joined minibar and output preview"><figcaption>ACTUAL INTERFACE · SAMPLE TRANSCRIPT</figcaption></figure>
    <footer>TURBO BY DEFAULT &nbsp; / &nbsp; MEDIUM REASONING &nbsp; / &nbsp; WINDOWS x64<span>Ctrl + Shift + Space</span></footer>
  </main></body></html>`);
  await page.locator('figure img').evaluate(image => image.decode());
  await page.evaluate(async () => { await document.fonts.ready; await new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))); });
  await page.screenshot({path:'screenshots/product-preview.png'});
  // Element bounds crop directly to the tested widget, without desktop/user data.
  await page.locator('figure img').screenshot({path:'screenshots/widget-crop.png'});
} finally {await browser.close();}
