const puppeteer = require('puppeteer-core');
const fs = require('fs');
const [,, infoFile, shots] = process.argv;
const wait = (ms) => new Promise((r) => setTimeout(r, ms));
(async () => {
  let info = '';
  for (let i = 0; i < 300 && !info.includes('\n'); i++) { await wait(200); try { info = fs.readFileSync(infoFile, 'utf8'); } catch {} }
  const [port, code] = info.trim().split('\n');
  const browser = await puppeteer.launch({
    executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    headless: true, acceptInsecureCerts: true,
    userDataDir: shots + '/profile',
    args: ['--ignore-certificate-errors', '--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream', '--no-first-run'],
    defaultViewport: { width: 420, height: 900, deviceScaleFactor: 2 },
  });
  const page = await browser.newPage();
  page.on('console', (m) => console.log('console:', m.type(), m.text()));
  page.on('response', (r) => { if (r.status() >= 400) console.log('HTTP', r.status(), r.url()); });
  page.on('pageerror', (e) => console.log('pageerror:', e.message));
  await page.goto(`https://127.0.0.1:${port}/#join=${code}&p=47820`, { waitUntil: 'networkidle0' });
  await page.screenshot({ path: shots + '/1-home.png' });
  await page.click('[data-goto="profiles"]');
  await page.type('#pname', 'Navigo');
  await page.click('#create');
  await page.waitForSelector('#go');           // écran de calibrage
  await page.screenshot({ path: shots + '/2-calib.png' });
  await page.click('#back'); await wait(200);   // → profils
  await page.click('#back'); await wait(200);   // → accueil
  await page.click('[data-goto="join"]');
  await page.waitForSelector('#code');
  console.log('code prérempli :', await page.$eval('#code', (e) => e.value));
  await page.click('#go');
  try {
    await page.waitForFunction(() => document.body.innerText.includes('Joueurs (2)'), { timeout: 15000 });
  } catch (e) {
    await page.screenshot({ path: shots + '/fail.png' });
    console.log('PAGE:', (await page.evaluate(() => document.body.innerText)).slice(0, 600).split(String.fromCharCode(10)).join(' | '));
    console.log('TOAST:', await page.$eval('#toast', (t) => (t.hidden ? '(aucun)' : t.textContent)));
    throw e;
  }
  await wait(500);
  await page.screenshot({ path: shots + '/3-lobby.png' });
  console.log('LOBBY OK:', (await page.$eval('main', (e) => e.innerText)).split('\n').slice(0, 6).join(' | '));
  const wasmResp = page.waitForResponse((r) => r.url().endsWith('.wasm'));
  await page.reload({ waitUntil: 'networkidle0' });
  const wr = await wasmResp;
  console.log('WASM HEADERS:', wr.status(), wr.headers()['content-encoding'], wr.headers()['etag']);
  await page.waitForFunction(() => document.body.innerText.includes('Joueurs (2)'), { timeout: 15000 });
  console.log('RELOAD OK:', (await page.$eval('main', (e) => e.innerText)).split('\n').slice(0, 3).join(' | '));
  await page.screenshot({ path: shots + '/4-reload.png' });
  const analysis = await page.evaluate(async () => {
    const m = await import('/web/youliar_web.js');
    await m.default();
    const sr = 48000, n = sr * 2, a = new Float32Array(n);
    for (let i = sr / 2; i < n; i++) a[i] = 0.2 * Math.sin(2 * Math.PI * 150 * i / sr) * (0.6 + 0.4 * Math.sin(2 * Math.PI * 4 * i / sr));
    const t0 = performance.now();
    const f = JSON.parse(m.analyze_audio(a, sr, 0));
    return { ms: Math.round(performance.now() - t0), speech: f.speech_detected, latency: f.latency_s.toFixed(2), f0: (55 * 2 ** (f.f0_mean_st / 12)).toFixed(1) };
  });
  console.log('WASM ANALYSE:', JSON.stringify(analysis));
  await browser.close();
})().catch((e) => { console.log('E2E FAIL', e.message); process.exit(1); });
