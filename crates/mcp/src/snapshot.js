(() => {
  const seen = new Set();
  let next = 1;
  const refs = new Map();
  window.__dotoriRefs = refs;
  const interactive = 'a[href],button,input,select,textarea,summary,[role=button],[role=link],[role=checkbox],[role=tab],[role=menuitem],[contenteditable=true]';
  const hidden = (el) => {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return true;
    if (style.display === 'contents') return false;
    const rect = el.getBoundingClientRect();
    return rect.width === 0 && rect.height === 0;
  };
  const label = (el) => {
    const text = (el.getAttribute('aria-label') || el.innerText || el.placeholder || (el.type === 'password' ? '' : el.value) || el.title || el.alt || el.name || '').trim();
    return text.replace(/\s+/g, ' ').slice(0, 120);
  };
  const kind = (el) => {
    const tag = el.tagName.toLowerCase();
    if (tag === 'a') return 'link';
    if (tag === 'input') return 'input ' + (el.type || 'text');
    return el.getAttribute('role') || tag;
  };
  const lines = [];
  const walk = (node) => {
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
      lines.push('[frame: ' + (el.title || el.src || 'unnamed') + ', not read]');
      return;
    }
    if (el.matches(interactive) && !seen.has(el)) {
      seen.add(el);
      const ref = next++;
      refs.set(ref, el);
      let line = '[' + ref + '] ' + kind(el) + ': ' + label(el);
      if (tag === 'a') line += ' (' + el.href + ')';
      if (tag === 'input' && el.type !== 'password' && el.value && el.value !== label(el)) line += ' = ' + el.value;
      if (el.type === 'checkbox' || el.type === 'radio') line += el.checked ? ' [checked]' : ' [ ]';
      lines.push(line);
      if (tag !== 'select') return;
    }
    if (/^h[1-6]$/.test(tag)) {
      lines.push('#'.repeat(Number(tag[1])) + ' ' + label(el));
      return;
    }
    for (const child of el.childNodes) walk(child);
    if (el.shadowRoot) for (const child of el.shadowRoot.childNodes) walk(child);
  };
  walk(document.body || document.documentElement);
  const merged = [];
  for (const line of lines) {
    if (merged.length && !line.startsWith('[') && !line.startsWith('#') && !merged[merged.length - 1].startsWith('[') && !merged[merged.length - 1].startsWith('#')) {
      merged[merged.length - 1] += ' ' + line;
    } else {
      merged.push(line);
    }
  }
  return 'Title: ' + document.title + '\nURL: ' + location.href + '\n\n' + merged.join('\n');
})()
