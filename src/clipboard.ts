/** Copying a grid so it lands in a spreadsheet as cells, not as one string.
 *
 *  Two flavours go on the clipboard at once. `text/plain` is tab-separated,
 *  which is what Excel, Numbers and Sheets parse into columns when you paste.
 *  `text/html` is a real `<table>`, which those apps prefer when present and
 *  which keeps the column split unambiguous even when a value contains a tab.
 */

/** Flatten a value for the plain-text flavour.
 *
 *  XML text content can contain tabs and newlines, and both are TSV record
 *  separators. RFC 4180 says quote such a field, but spreadsheet *paste*
 *  parsers handle quoted multi-line fields far less reliably than file
 *  import does, and one stray newline shifts every following row. Collapsing
 *  whitespace guarantees one record per row in every target. The `text/html`
 *  flavour carries the value exactly, and that is what Excel, Numbers and
 *  Sheets prefer when both are on the clipboard. */
function tsvField(v: string): string {
  return v.replace(/[\t\r\n]+/g, " ");
}

function escapeHtml(v: string): string {
  return v.replace(
    /[&<>"]/g,
    (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]!,
  );
}

export interface Table {
  header: string[];
  rows: string[][];
}

export function toTsv(t: Table): string {
  const lines = [t.header.map(tsvField).join("\t")];
  for (const r of t.rows) lines.push(r.map(tsvField).join("\t"));
  return lines.join("\n");
}

export function toHtml(t: Table): string {
  const head = t.header.map((h) => `<th>${escapeHtml(h)}</th>`).join("");
  const body = t.rows
    .map((r) => `<tr>${r.map((c) => `<td>${escapeHtml(c)}</td>`).join("")}</tr>`)
    .join("");
  return `<table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table>`;
}

/** Write both flavours, degrading as the environment allows. */
export async function copyTable(t: Table): Promise<boolean> {
  return copyRaw(toTsv(t), toHtml(t));
}

/** Same, for content already serialised elsewhere — the backend builds the
 *  full-group export so the clipboard is not limited by the grid's row cap. */
export async function copyRaw(
  text: string,
  html: string | null,
): Promise<boolean> {

  try {
    if (html && typeof ClipboardItem !== "undefined" && navigator.clipboard?.write) {
      await navigator.clipboard.write([
        new ClipboardItem({
          "text/plain": new Blob([text], { type: "text/plain" }),
          "text/html": new Blob([html], { type: "text/html" }),
        }),
      ]);
      return true;
    }
  } catch {
    // Fall through — some webviews advertise write() but reject the call.
  }

  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    // Fall through.
  }

  // Last resort for webviews without the async clipboard API.
  const ta = document.createElement("textarea");
  ta.value = text;
  ta.setAttribute("readonly", "");
  ta.style.cssText = "position:fixed;top:-1000px;opacity:0";
  document.body.appendChild(ta);
  ta.select();
  let ok = false;
  try {
    ok = document.execCommand("copy");
  } catch {
    ok = false;
  }
  ta.remove();
  return ok;
}
