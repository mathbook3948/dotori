function (...typed) {
  const prop = (el, name) => {
    for (let proto = Object.getPrototypeOf(el); proto; proto = Object.getPrototypeOf(proto)) {
      const found = Object.getOwnPropertyDescriptor(proto, name);
      if (found && found.get) return found.get.call(el);
    }
    return undefined;
  };
  const masked = new Set(typed);
  for (const el of typed) {
    const form = prop(el, 'form');
    if (!form) continue;
    for (const other of prop(form, 'elements')) masked.add(other);
  }
  const valueOf = (el) => (el.type === 'password' || masked.has(el) ? '' : el.value);
  const seen = new Set();
  let next = 1;
  const refs = new Map();
  globalThis.__refs = refs;
  const interactive = 'a[href],button,input,select,textarea,summary,[role=button],[role=link],[role=checkbox],[role=tab],[role=menuitem],[contenteditable=true]';
  const hidden = (el) => {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return true;
    if (style.display === 'contents') return false;
    const rect = el.getBoundingClientRect();
    return rect.width === 0 && rect.height === 0;
  };
  const label = (el) => {
    const text = (el.getAttribute('aria-label') || el.innerText || el.placeholder || valueOf(el) || el.title || el.alt || el.name || '').trim();
    return text.replace(/\s+/g, ' ').slice(0, 120);
  };
  const kind = (el) => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'input') return 'input ' + (el.type || 'text');
    return el.getAttribute('role') || tag;
  };
  const lines = [];
  const walk = (node, framed) => {
    if (node.nodeType === Node.TEXT_NODE) {
      const text = node.textContent.replace(/\s+/g, ' ').trim();
      if (text) lines.push(text);
      return;
    }
    if (node.nodeType !== Node.ELEMENT_NODE) return;
    const el = node;
    const tag = el.tagName.toLowerCase();
    if (['script', 'style', 'noscript', 'template', 'svg', 'head'].includes(tag)) return;
    if (hidden(el)) return;
    if (tag === 'iframe') {
      const name = el.title || el.src || 'unnamed';
      const inner = el.contentDocument && el.contentDocument.body;
      if (framed) {
        lines.push('[frame: ' + name + ', inside a frame, not read]');
        return;
      }
      if (!inner) {
        lines.push('[frame: ' + name + ', not read]');
        return;
      }
      lines.push('[frame: ' + name + ']');
      for (const child of inner.childNodes) walk(child, true);
      lines.push('[end of frame]');
      return;
    }
    if (framed && el.matches(interactive)) {
      lines.push('- ' + kind(el) + ': ' + label(el));
      if (tag !== 'select') return;
    }
    if (!framed && el.matches(interactive) && !seen.has(el)) {
      seen.add(el);
      const ref = next++;
      refs.set(ref, el);
      let line = '[' + ref + '] ' + kind(el) + ': ' + label(el);
      if (tag === 'a') line += ' (' + el.href + ')';
      if (tag === 'input' && valueOf(el) && valueOf(el) !== label(el)) line += ' = ' + valueOf(el);
      if (el.type === 'checkbox' || el.type === 'radio') line += el.checked ? ' [checked]' : ' [ ]';
      lines.push(line);
      if (tag !== 'select') return;
    }
    if (/^h[1-6]$/.test(tag)) {
      lines.push('#'.repeat(Number(tag[1])) + ' ' + label(el));
      return;
    }
    for (const child of el.childNodes) walk(child, framed);
    if (el.shadowRoot) for (const child of el.shadowRoot.childNodes) walk(child, framed);
  };
  walk(document.body || document.documentElement, false);
  const merged = [];
  for (const line of lines) {
    const apart = (text) => text.startsWith('[') || text.startsWith('#') || text.startsWith('- ');
    if (merged.length && !apart(line) && !apart(merged[merged.length - 1])) {
      merged[merged.length - 1] += ' ' + line;
    } else {
      merged.push(line);
    }
  }
  return 'Title: ' + document.title + '\nURL: ' + location.href + '\n\n' + merged.join('\n');
}
