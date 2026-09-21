const { chromium } = require('playwright');
const { spawn, execFileSync } = require('node:child_process');
const { mkdtempSync, writeFileSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const path = require('node:path');
const readline = require('node:readline');
const assert = require('node:assert/strict');
const root = path.resolve(__dirname, '../..');
const modifier = process.platform === 'darwin' ? 'Meta' : 'Control';
const temp = mkdtempSync(path.join(tmpdir(), 'xmlrows-gui-'));
let backend, browser, server;
(async () => {
  execFileSync('python3', [path.join(__dirname, 'backend.py'), temp], { cwd: root });
  execFileSync('cargo', ['build', '--quiet', '--manifest-path', path.join(temp, 'Cargo.toml')], { cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_TARGET_DIR: path.join(root, 'src-tauri/target/gui-tests') } });
  const firstPath = path.join(temp, 'first.xml');
  const secondPath = path.join(temp, 'second.xml');
  const second = '<root><row><name>New file</name></row><row><name>Another</name></row></root>';
  writeFileSync(firstPath, '<root><row><name>Before</name></row><row><name>Second</name></row></root>');
  writeFileSync(secondPath, second);
  backend = spawn(path.join(root, 'src-tauri/target/gui-tests/debug/xmlrows-design-backend'), [firstPath]);
  const pending = [];
  readline.createInterface({ input: backend.stdout }).on('line', line => {
    const value = JSON.parse(line), task = pending.shift();
    if (value?.__error) task.reject(new Error(value.__error)); else task.resolve(value);
  });
  const invoke = (cmd, args = {}) => new Promise((resolve, reject) => {
    pending.push({ resolve, reject }); backend.stdin.write(JSON.stringify({ cmd, args }) + '\n');
  });
  const url = 'http://localhost:1421';
  server = spawn(process.execPath, [path.join(root, 'node_modules/vite/bin/vite.js'), '--port', '1421', '--strictPort'], { cwd: root });
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Vite startup timeout')), 10000);
    server.stdout.on('data', data => { if (data.toString().includes('1421')) { clearTimeout(timer); resolve(); } });
    server.on('exit', code => { clearTimeout(timer); reject(new Error(`Vite exited: ${code}`)); });
  });
  browser = await chromium.launch({ headless: true, ...(process.env.CHROME_PATH ? { executablePath: process.env.CHROME_PATH } : {}) });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors = [], messages = [];
  page.on('pageerror', e => errors.push(e.message));
  let saveFailure = false, saveCalls = 0;
  await page.exposeFunction('__testInvoke', async (cmd, args) => {
    if (cmd === 'plugin:dialog|open') return secondPath;
    if (cmd === 'plugin:dialog|save') return firstPath;
    if (cmd.startsWith('plugin:')) { messages.push(args); return null; }
    if (cmd === 'save_file') { saveCalls++; if (saveFailure) throw new Error('Simulated save failure'); }
    return invoke(cmd, args);
  });
  await page.addInitScript(() => {
    window.__TAURI_INTERNALS__ = { invoke: (cmd, args) => window.__testInvoke(cmd, args) };
    Object.defineProperty(window, 'ClipboardItem', { value: undefined });
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText: async (text) => { window.__copiedText = text; } },
    });
  });
  await page.goto(url);
  await page.waitForSelector('table.rows td.cell');
  assert.equal(await page.locator('#dirty-mark').isVisible(), false, 'opening a document should not mark it dirty');

  assert.equal(await page.locator('#btn-locate').isVisible(), false);
  await page.locator('#btn-about').click();
  assert.equal(await page.locator('#about-dialog').isVisible(), true);
  assert.ok((await page.locator('#about-dialog').innerText()).includes('Martin Valland'));
  assert.ok((await page.locator('#about-dialog').innerText()).includes('© 2026 Bråtet Software AS'));
  await page.locator('#about-dialog summary').filter({hasText:'License terms'}).click();
  assert.ok((await page.locator('#about-license-text').innerText()).includes('MIT License'));
  await page.locator('#about-dialog summary').filter({hasText:'Open-source notices'}).click();
  await page.waitForFunction(()=>document.querySelector('#about-notices-text').textContent.includes('@codemirror'));
  await page.keyboard.press('Escape');
  assert.equal(await page.locator('#about-dialog').isVisible(), false);
  await page.locator('#tables-only').uncheck();
  assert.equal(await page.locator('#btn-locate').isVisible(), true);
  await page.locator('#tables-only').check();
  assert.equal(await page.locator('#btn-locate').isVisible(), false);
  console.log('PASS conditional Find in tree and About credits, copyright and bundled licenses');
  async function show(text) {
    await invoke('open_file', { path: firstPath });
    await invoke('set_text', { text });
    await page.reload();
    await page.locator("#tables-only").uncheck();
    await page.locator('#tree [data-id="0"] .tag').click();
    await page.waitForSelector('table.rows td.cell');
  }
  const cell = () => page.locator('[data-cell="0:0:0"]');
  async function edit(value) {
    await cell().dblclick();
    await page.locator('.cell-edit').fill(value);
    await page.locator('.cell-edit').press('Enter');
    await page.waitForFunction(() => !document.querySelector('.cell-edit'));
  }
  const long = '  ' + 'ø😀'.repeat(320) + '\nlast line  ';
  await show(`<root>\n${'<!-- spacer -->\n'.repeat(150)}<row><name>${long}</name></row><row><name>Second</name></row></root>`);
  await cell().click();
  const cellColorBeforeEdit = await cell().evaluate((el) => getComputedStyle(el).backgroundColor);
  const cellBorderBeforeEdit = await cell().evaluate((el) => getComputedStyle(el).boxShadow);
  const cellBoxBeforeEdit = await cell().boundingBox();
  await cell().dblclick();
  const cellBoxDuringEdit = await page.locator('.cell-edit').locator('..').boundingBox();
  assert.ok(cellBoxBeforeEdit && cellBoxDuringEdit);
  assert.ok(Math.abs(cellBoxBeforeEdit.width - cellBoxDuringEdit.width) < 0.5, 'editing preserves cell width');
  assert.ok(Math.abs(cellBoxBeforeEdit.height - cellBoxDuringEdit.height) < 0.5, 'editing preserves cell height');
  assert.equal(await page.locator('.cell-edit').inputValue(), long, 'edit full text including whitespace and Unicode');
  assert.equal(await page.locator('.cell-edit').evaluate((el) => getComputedStyle(el).outlineStyle), 'none');
  assert.equal(
    await page.locator('.cell-edit').evaluate((el) => getComputedStyle(el).backgroundColor),
    cellColorBeforeEdit,
    'editing preserves the selected cell color',
  );
  assert.equal(
    await page.locator('.cell-edit').evaluate((el) => getComputedStyle(el).boxShadow),
    cellBorderBeforeEdit,
    'the visible editor preserves the selected cell border',
  );
  await page.waitForTimeout(100);
  const scroll = await page.locator('.cm-scroller').evaluate(e => e.scrollTop);
  await page.locator('.cell-edit').fill(long + 'X');
  await page.locator('.cell-edit').press('Enter');
  await page.waitForFunction(() => !document.querySelector('.cell-edit'));
  assert.ok((await invoke('document_text')).includes(long + 'X'));
  await page.waitForTimeout(100);
  assert.ok(Math.abs(await page.locator('.cm-scroller').evaluate(e => e.scrollTop) - scroll) < 35, 'source position preserved');
  console.log('PASS full-value edit and source position');

  await show('<root><row><name>Before</name></row><row><name>Second</name></row></root>');
  await page.locator('.cm-content').click();
  await page.keyboard.press(`${modifier}+a`);
  await page.keyboard.insertText('<root><row><name>After</name></row><row><name>Second</name></row><row><name>Third</name></row></root>');
  await page.waitForFunction(() => document.querySelector('[data-cell="0:0:0"]')?.textContent === 'After');
  assert.equal(await page.locator('tr[data-row]').count(), 3);
  assert.equal(await page.locator('#tree .tag').filter({hasText:/^row$/}).count(), 3);
  await edit('Edited again');
  assert.ok((await invoke('document_text')).includes('<name>Edited again</name>'));
  console.log('PASS direct source edit refreshes table, tree and edit offsets');

  await page.locator('#btn-open').click();
  await page.locator('#unsaved-dialog button[value=cancel]').click();
  assert.equal(await page.locator('#filename').innerText(), 'first.xml');
  assert.ok((await invoke('document_text')).includes('Edited again'));
  await page.locator('#btn-open').click();
  saveFailure = true;
  await page.locator('#unsaved-dialog button[value=save]').click();
  await page.waitForTimeout(200);
  assert.equal(await page.locator('#filename').innerText(), 'first.xml');
  assert.ok(messages.length > 0);
  saveFailure = false;
  await page.locator('#btn-open').click();
  await page.locator('#unsaved-dialog button[value=save]').click();
  await page.waitForFunction(() => document.querySelector('#filename').textContent === 'second.xml');
  assert.ok(readFileSync(firstPath, 'utf8').includes('Edited again'));
  assert.equal(saveCalls, 2);
  await page.locator('.cm-content').click();
  await page.keyboard.press(`${modifier}+z`);
  await page.waitForTimeout(250);
  assert.equal(await invoke('document_text'), second, 'undo cannot restore previous file');
  assert.equal(await page.locator('#dirty-mark').isVisible(), false);
  console.log('PASS cancel, failed save, save before open, fresh undo history');

  await edit('Discard this');
  await page.locator('#btn-open').click();
  await page.locator('#unsaved-dialog button[value=discard]').click();
  await page.waitForFunction(() => document.querySelector('[data-cell="0:0:0"]')?.textContent === 'New file');
  assert.equal(await invoke('document_text'), second);
  assert.equal(saveCalls, 2, 'discard does not save');
  console.log('PASS discard changes');

  await show('<root>\n' + Array.from({ length: 450 }, (_, i) => `<row><name>${i}</name></row>\n`).join('') + '</root>');
  await page.waitForSelector('#tree button[data-more]');
  assert.ok(await page.locator('#tree').evaluate(e => {
    const more = e.querySelector('[data-more]');
    const rows = e.querySelectorAll('[data-id]');
    return !!(rows[rows.length - 1].compareDocumentPosition(more) & Node.DOCUMENT_POSITION_FOLLOWING);
  }), 'load-more is after loaded descendants');
  await page.locator('#tree button[data-more]').click();
  await page.waitForFunction(() => !document.querySelector('#tree [data-more]'));
  assert.equal(await page.locator('#tree .tag').filter({ hasText: /^row$/ }).count(), 450);
  console.log('PASS load-more button placement and pagination with whitespace');

  await show('<root><header>metadata</header><order id="a"><item>one</item><item>two</item></order><order id="b"><item>three</item><item>four</item><item>five</item></order><customer>A</customer><customer>B</customer><customer>C</customer></root>');
  await page.locator('#tables-only').check();
  await page.waitForFunction(() => document.querySelectorAll('.table-entry').length === 2);
  const entries = await page.locator('.table-entry').allTextContents();
  assert.ok(entries[0].includes('order') && entries[0].includes('2 rows'));
  assert.ok(entries[1].includes('customer') && entries[1].includes('3 rows'));
  assert.equal(await page.locator('.table-entry-name').filter({hasText:/^item$/}).count(), 0);
  assert.equal(await page.locator('#tree .count').count(), 0);
  await page.locator('.table-entry').nth(1).click();
  await page.waitForFunction(() => document.querySelectorAll('table.rows').length === 1 && document.querySelector('table.rows')?.textContent.includes('A'));
  assert.equal(await page.locator('tr[data-row]').count(), 3);
  await page.locator('.table-entry').nth(0).click();
  await page.waitForFunction(() => document.querySelector('table.rows')?.textContent.includes('one'));
  assert.equal(await page.locator('tr[data-row]').count(), 2);
  await page.locator('td.cell').filter({hasText:/^one$/}).dblclick();
  await page.locator('.cell-edit').fill('edited item');
  await page.locator('.cell-edit').press('Enter');
  await page.waitForFunction(() => document.querySelector('table.rows')?.textContent.includes('edited item'));
  assert.equal(await page.locator('table.rows').count(), 1);
  assert.ok((await page.locator('.table-entry.selected').innerText()).includes('order'));
  await page.locator('#tables-only').uncheck();
  assert.equal(await page.locator('.table-entry').count(), 0);
  console.log('PASS table navigator names, row counts, separate sibling groups, hidden nested tables and selection after editing');

  await invoke('open_file', { path: firstPath });
  await invoke('set_text', { text: '<root><other><x>A</x><x>B</x></other><wrapper><row><name>One</name></row><row><name>Two</name></row></wrapper></root>' });
  await page.reload();
  await page.locator('#tables-only').check();
  await page.waitForFunction(() => document.querySelectorAll('.table-entry').length === 2);
  await page.locator('.table-entry').filter({hasText:/^row2 rows$/}).click();
  await page.waitForFunction(() => document.querySelector('#detail-title').textContent.startsWith('row'));
  await page.locator('#btn-format').click();
  await page.waitForFunction(() => document.querySelector('#detail-title').textContent.startsWith('row') && document.querySelectorAll('tr[data-row]').length === 2);
  assert.equal(await page.locator('#btn-detail').isEnabled(), true, 'formatting keeps the table available');
  assert.equal(await page.locator('#detail-toggle-label').innerText(), 'Hide table');
  assert.ok((await page.locator('table.rows').innerText()).includes('One'));
  assert.ok((await invoke('document_text')).includes('\n  <wrapper>'), 'formatting updates the backend document');
  console.log('PASS formatting preserves the selected table and its availability');

  await show('<root><header>metadata</header><order id="a"><item>edited item</item><item>two</item></order><order id="b"><item>three</item><item>four</item><item>five</item></order><customer>A</customer><customer>B</customer><customer>C</customer></root>');

  async function caretAt(value) {
    await page.evaluate(async (value) => {
      const { EditorView } = await import('/node_modules/.vite/deps/@codemirror_view.js');
      const editor = EditorView.findFromDOM(document.querySelector('.cm-editor'));
      const pos = editor.state.doc.toString().indexOf(value) + 1;
      editor.focus();
      editor.dispatch({ selection: { anchor: pos }, effects: [EditorView.scrollIntoView(pos, { y: 'center' })] });
    }, value);
  }
  await page.locator('#tables-only').check();
  assert.equal(await page.locator('.table-entry-path').count(), 0);
  await caretAt('three');
  await page.waitForFunction(() => document.querySelector('td.focused')?.textContent === 'three');
  assert.equal(await page.locator('table.rows').count(), 1);
  assert.ok((await page.locator('.table-entry.selected').innerText()).includes('order'));
  assert.ok(await page.locator('.cm-content').evaluate(e => e.contains(document.activeElement) || e === document.activeElement));
  await page.locator('th[data-sort]').first().click();
  await caretAt('four');
  await page.waitForFunction(() => document.querySelector('td.focused')?.textContent === 'four');
  await caretAt('metadata');
  await page.waitForFunction(() => document.querySelector('#main-pane').dataset.detail === 'collapsed');
  await caretAt('edited item');
  await page.waitForFunction(() => document.querySelector('td.focused')?.textContent === 'edited item');
  assert.equal(await page.locator('#main-pane').getAttribute('data-detail'), 'open');
  await show('<root>\n' + Array.from({length: 2205}, (_, i) => `<row code="id-${i}"><name>value-${i}</name></row>\n`).join('') + '</root>');
  await caretAt('value-2201');
  await page.waitForFunction(() => document.querySelector('td.focused')?.textContent === 'value-2201');
  assert.equal(await page.locator('#opt-rows').inputValue(), '10000');
  assert.ok(await page.locator('td.focused').isVisible());
  await caretAt('id-2201');
  await page.waitForFunction(() => document.querySelector('td.focused')?.textContent === 'id-2201');
  console.log('PASS source caret selects nested values, attributes, sorted cells and rows beyond initial display limit without stealing focus');

  const orderA = '<order><name>first order</name></order>';
  const orderB = '<order><name>second order</name></order>';
  const customerA = '<customer>first customer</customer>';
  const customerB = '<customer>second customer</customer>';
  await show(`<root title="😀">${orderA}${customerA}<note>not a row</note>${orderB}${customerB}</root>`);
  await page.locator('#tables-only').check();
  await page.locator('.table-entry').filter({hasText:'order'}).click();

  await page.waitForFunction(() => document.querySelector('#detail-title').textContent.startsWith('order'));
  assert.equal(await page.locator('.cm-scope, .cm-focus').count(), 0);
  const xml = await invoke('document_text');
  await page.locator('td.cell').filter({hasText:/^first order$/}).click();
  assert.equal(await page.locator('.cm-focus').innerText(), 'first order');
  assert.equal(await page.locator('.cm-scope').count(), 0);
  await page.locator('[data-highlight-row]').first().click();
  assert.equal((await page.locator('.cm-focus').allTextContents()).join(''), orderA);
  await page.locator('.table-entry').filter({hasText:'customer'}).click();
  await page.waitForFunction(() => document.querySelector('#detail-title').textContent.startsWith('customer'));
  assert.equal(await page.locator('.cm-scope, .cm-focus').count(), 0);
  assert.equal(await page.evaluate(async()=>{
    const {EditorView}=await import('/node_modules/.vite/deps/@codemirror_view.js');
    return EditorView.findFromDOM(document.querySelector('.cm-editor')).state.selection.main.head;
  }), xml.indexOf(customerA));
  await caretAt('second customer');
  await page.waitForFunction(() => document.querySelector('td.focused')?.textContent === 'second customer');
  assert.equal(await page.locator('.cm-scope, .cm-focus').count(), 0);
  console.log('PASS table selection navigates without highlighting; row/cell clicks highlight their XML; editor caret selects only the table cell');

  await show('<root><row><name>one</name></row><row><name>two</name></row><row><name>three</name></row></root>');
  for (let row = 0; row < 3; row++) {
    await page.locator(`[data-highlight-row="0:${row}"]`).click();
    assert.equal(
      await page.locator(`[data-highlight-row="0:${row}"]`).evaluate((el) => getComputedStyle(el).backgroundColor),
      await page.locator(`[data-cell="0:${row}:0"]`).evaluate((el) => getComputedStyle(el).backgroundColor),
      `row ${row + 1} number uses the same highlight as its cells`,
    );
  }
  console.log('PASS row-number highlight is consistent across odd and even table stripes');

  await page.locator('[data-cell="0:0:0"]').click();
  await page.locator('[data-cell="0:2:0"]').click({ modifiers: ['Shift'] });
  assert.equal(await page.locator('td.cell.focused').count(), 1);
  assert.equal(await page.locator('td.cell.picked').count(), 0, 'Shift-click does not create a multi-cell selection');

  await page.locator('[data-highlight-row="0:0"]').click();
  await page.locator('[data-highlight-row="0:1"]').click({ modifiers: ['Shift'] });
  const selectedXml = (await page.locator('.cm-focus').allTextContents()).join('');
  assert.ok(selectedXml.includes('<row><name>one</name></row>'));
  assert.ok(selectedXml.includes('<row><name>two</name></row>'));
  assert.ok(!selectedXml.includes('<row><name>three</name></row>'), 'only selected rows are highlighted in XML');
  await page.locator('[data-copy="0"]').click();
  await page.waitForFunction(() => window.__copiedText?.includes('one'));
  const selectedRowsCopy = await page.evaluate(() => window.__copiedText);
  assert.ok(selectedRowsCopy.includes('one') && selectedRowsCopy.includes('two'));
  assert.ok(!selectedRowsCopy.includes('three'), 'Copy button excludes unselected rows');
  assert.equal(await page.locator('[data-highlight-row].row-selected').count(), 2, 'Copy preserves the row selection');
  await page.locator('.group-heading h2').click();
  assert.equal(await page.locator('[data-highlight-row].row-selected').count(), 0);
  assert.equal(await page.locator('.cm-focus').count(), 0, 'clicking outside the table clears XML highlights');
  console.log('PASS cell selection stays singular; Copy and XML highlights respect selected rows; outside click clears selection');

  const dragStart = await page.locator('[data-cell="0:0:0"]').boundingBox();
  const dragEnd = await page.locator('[data-cell="0:2:0"]').boundingBox();
  assert.ok(dragStart && dragEnd);
  await page.mouse.move(dragStart.x + dragStart.width / 2, dragStart.y + dragStart.height / 2);
  await page.mouse.down();
  await page.mouse.move(dragEnd.x + dragEnd.width / 2, dragEnd.y + dragEnd.height / 2, { steps: 8 });
  await page.mouse.up();
  assert.equal(await page.evaluate(() => window.getSelection()?.toString() ?? ''), '');
  assert.equal(await page.locator('[data-cell="0:1:0"]').evaluate((el) => getComputedStyle(el).userSelect), 'none');
  console.log('PASS dragging across table cells cannot create a browser text selection');

  const firstRow = '<n:row id="a"><n:name>Zebra</n:name><n:details><!-- keep --><![CDATA[a < b]]><n:x flag="yes"/></n:details></n:row>';
  const secondRow = '<n:row id="b"><n:name>Apple</n:name></n:row>';
  const original = `<root xmlns:n="urn:test">\n  ${firstRow}\n  ${secondRow}\n</root>`;
  await show(original);
  assert.equal(await page.locator('[data-duplicate]').isDisabled(), true);
  await page.locator('[data-highlight-row="0:0"]').click();
  await page.locator('[data-highlight-row="0:1"]').click({ modifiers: ['Shift'] });
  assert.equal(await page.locator('[data-highlight-row].row-selected').count(), 2);
  assert.equal(await page.locator('[data-duplicate]').isDisabled(), true, 'multiple rows cannot be duplicated as one row');
  await page.locator('[data-highlight-row="0:0"]').click({ modifiers: [modifier] });
  assert.equal(await page.locator('[data-highlight-row].row-selected').count(), 1);
  assert.equal(await page.locator('[data-duplicate]').isDisabled(), false);
  assert.equal(
    await page.locator('[data-highlight-row="0:1"]').evaluate((el) => getComputedStyle(el).backgroundColor),
    await page.locator('[data-cell="0:1:0"]').evaluate((el) => getComputedStyle(el).backgroundColor),
    'selected row number uses the same highlight as its cells on striped rows',
  );
  await page.locator('th[data-sort]').filter({hasText:'n:name'}).click();
  await page.waitForSelector('th.sorted');
  assert.equal(await page.locator('[data-highlight-row="0:0"]').getAttribute('class'), 'rownum row-selected', 'row selection follows its XML node when sorted');
  await page.locator('td.cell').filter({hasText:/^Zebra$/}).click();
  await page.locator('[data-duplicate]').click();
  await page.waitForFunction(() => document.querySelectorAll('tr[data-row]').length === 3);
  const duplicated = `<root xmlns:n="urn:test">\n  ${firstRow}\n  ${firstRow}\n  ${secondRow}\n</root>`;
  assert.equal(await invoke('document_text'), duplicated);
  assert.equal((await invoke('document_info')).errors.length, 0);
  assert.equal(await page.locator('[data-highlight-row="0:1"]').getAttribute('class'), 'rownum row-selected');
  assert.equal((await page.locator('.cm-focus').allTextContents()).join(''), firstRow);
  await page.keyboard.press(`${modifier}+z`);
  await page.waitForFunction(() => document.querySelectorAll('tr[data-row]').length === 2);
  assert.equal(await invoke('document_text'), original);
  await page.keyboard.press(`${modifier}+Shift+z`);
  await page.waitForFunction(() => document.querySelectorAll('tr[data-row]').length === 3);
  assert.equal(await invoke('document_text'), duplicated);
  await page.locator('[data-cell="0:1:1"]').dblclick();
  await page.locator('.cell-edit').fill('New row name');
  await page.locator('.cell-edit').press('Enter');
  await page.waitForFunction(() => !document.querySelector('.cell-edit'));
  assert.equal(await invoke('document_text'), duplicated.replace(firstRow+'\n  '+firstRow, firstRow+'\n  '+firstRow.replace('Zebra','New row name')));
  await show('<root><row id="a"/><row id="b"/></root>');
  await page.locator('[data-cell="0:0:0"]').click();
  await page.locator('[data-duplicate]').click();
  await page.waitForFunction(() => document.querySelectorAll('tr[data-row]').length === 3);
  assert.equal(await invoke('document_text'), '<root><row id="a"/><row id="a"/><row id="b"/></root>');
  await show('<root>'+Array.from({length:500},(_,i)=>`<row id="${i}"/>`).join('')+'</root>');
  await page.locator('#table-options summary').click();
  await page.locator('#opt-rows').selectOption('500');
  await page.locator('[data-scroll]').evaluate(e=>{e.scrollTop=e.scrollHeight;});
  await page.locator('[data-highlight-row="0:499"]').click();
  await page.locator('[data-duplicate]').click();
  await page.waitForSelector('[data-highlight-row="0:500"].row-selected');
  assert.equal((await invoke('document_info')).errors.length, 0);
  console.log('PASS multi-row range/toggle selection, sorted selection, duplicate row, namespaces/comments/CDATA, new-row edit, undo/redo, self-closing rows and row-limit boundary');
  assert.deepEqual(errors, []);
})().catch(err => { console.error(err); process.exitCode = 1; }).finally(async () => {
  await browser?.close(); backend?.kill(); server?.kill(); rmSync(temp, { recursive: true, force: true });
});
