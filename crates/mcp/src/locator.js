(() => {
  const implicitRole = (el) => {
    const tag = el.tagName.toLowerCase();
    const type = (el.getAttribute('type') || '').toLowerCase();
    if (tag === 'a' && el.hasAttribute('href')) return 'link';
    if (tag === 'button') return 'button';
    if (tag === 'input') {
      if (['button', 'submit', 'reset', 'image'].includes(type)) return 'button';
      if (type === 'checkbox') return 'checkbox';
      if (type === 'radio') return 'radio';
      if (type === 'range') return 'slider';
      if (type === 'search') return 'searchbox';
      if (type === 'hidden') return '';
      return 'textbox';
    }
    if (tag === 'textarea') return 'textbox';
    if (tag === 'select') return el.multiple || el.size > 1 ? 'listbox' : 'combobox';
    if (tag === 'option') return 'option';
    if (/^h[1-6]$/.test(tag)) return 'heading';
    if (tag === 'img') return 'img';
    if (tag === 'ul' || tag === 'ol') return 'list';
    if (tag === 'li') return 'listitem';
    if (tag === 'nav') return 'navigation';
    if (tag === 'main') return 'main';
    if (tag === 'form') return 'form';
    if (tag === 'table') return 'table';
    if (tag === 'tr') return 'row';
    if (tag === 'td') return 'cell';
    if (tag === 'th') return 'columnheader';
    if (tag === 'dialog') return 'dialog';
    if (tag === 'summary') return 'button';
    return '';
  };
  const role = (el) => (el.getAttribute('role') || implicitRole(el)).split(' ')[0];
  const clean = (text) => (text || '').replace(/\s+/g, ' ').trim();
  const labelsOf = (el) => {
    const found = [];
    if (el.id) for (const label of document.querySelectorAll('label[for="' + CSS.escape(el.id) + '"]')) found.push(label);
    const wrapping = el.closest('label');
    if (wrapping) found.push(wrapping);
    return found;
  };
  const name = (el) => {
    const labelled = el.getAttribute('aria-labelledby');
    if (labelled) {
      const text = labelled.split(/\s+/).map((id) => document.getElementById(id)).filter(Boolean).map((node) => node.innerText).join(' ');
      if (clean(text)) return clean(text);
    }
    const aria = el.getAttribute('aria-label');
    if (clean(aria)) return clean(aria);
    const tag = el.tagName.toLowerCase();
    if (['input', 'textarea', 'select'].includes(tag)) {
      const labels = labelsOf(el).map((label) => label.innerText).join(' ');
      if (clean(labels)) return clean(labels);
      if (tag === 'input' && ['button', 'submit', 'reset'].includes(el.type)) return clean(el.value);
      return clean(el.getAttribute('placeholder') || el.getAttribute('title'));
    }
    if (tag === 'img') return clean(el.getAttribute('alt') || el.getAttribute('title'));
    return clean(el.innerText || el.textContent || el.getAttribute('title'));
  };
  const matches = (actual, wanted, exact) => {
    if (wanted && typeof wanted === 'object' && 'regex' in wanted) return new RegExp(wanted.regex, wanted.flags).test(actual);
    const want = clean(String(wanted));
    return exact ? actual === want : actual.toLowerCase().includes(want.toLowerCase());
  };
  const visible = (el) => {
    if (!el.isConnected) return false;
    const style = getComputedStyle(el);
    if (style.visibility === 'hidden' || style.display === 'none') return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  const all = (root) => {
    const found = [];
    const walk = (node) => {
      for (const child of node.children || []) {
        found.push(child);
        walk(child);
        if (child.shadowRoot) walk(child.shadowRoot);
      }
    };
    walk(root);
    return found;
  };
  const css = (root, selector) => {
    if (selector.startsWith('ref=')) {
      const el = globalThis.__refs && globalThis.__refs.get(Number(selector.slice(4)));
      return el && el.isConnected ? [el] : [];
    }
    if (selector.startsWith('text=')) return byText(root, selector.slice(5).replace(/^"|"$/g, ''), selector.slice(5).startsWith('"'));
    if (selector.startsWith('//') || selector.startsWith('xpath=')) {
      const path = selector.startsWith('xpath=') ? selector.slice(6) : selector;
      const result = document.evaluate(path, root, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
      const found = [];
      for (let i = 0; i < result.snapshotLength; i++) found.push(result.snapshotItem(i));
      return found;
    }
    const plain = Array.from(root.querySelectorAll(selector));
    if (plain.length) return plain;
    return all(root).filter((el) => el.shadowRoot).flatMap((host) => Array.from(host.shadowRoot.querySelectorAll(selector)));
  };
  const byText = (root, wanted, exact) => {
    const hits = all(root).filter((el) => {
      if (['SCRIPT', 'STYLE', 'NOSCRIPT', 'TEMPLATE'].includes(el.tagName)) return false;
      return matches(clean(el.innerText || el.textContent), wanted, exact);
    });
    return hits.filter((el) => !hits.some((other) => other !== el && el.contains(other)));
  };
  const step = (roots, part) => {
    const each = (pick) => roots.flatMap(pick);
    if ('css' in part) return each((root) => css(root, part.css));
    if ('role' in part) {
      return each((root) => all(root).filter((el) => {
        if (role(el) !== part.role) return false;
        if (part.name !== undefined && !matches(name(el), part.name, part.exact)) return false;
        if (part.level && role(el) === 'heading' && Number(el.tagName[1] || el.getAttribute('aria-level')) !== part.level) return false;
        return part.includeHidden || visible(el);
      }));
    }
    if ('text' in part) return each((root) => byText(root, part.text, part.exact));
    if ('label' in part) {
      return each((root) => all(root).filter((el) => {
        if (!['INPUT', 'TEXTAREA', 'SELECT'].includes(el.tagName) && !el.isContentEditable) return false;
        const labels = [el.getAttribute('aria-label') || '', ...labelsOf(el).map((label) => label.innerText)];
        return labels.some((label) => matches(clean(label), part.label, part.exact));
      }));
    }
    for (const [key, attribute] of [['placeholder', 'placeholder'], ['testId', 'data-testid'], ['alt', 'alt'], ['title', 'title']]) {
      if (key in part) return each((root) => all(root).filter((el) => el.hasAttribute(attribute) && matches(clean(el.getAttribute(attribute)), part[key], part.exact)));
    }
    if ('nth' in part) {
      const index = part.nth < 0 ? roots.length + part.nth : part.nth;
      return roots[index] ? [roots[index]] : [];
    }
    if ('hasText' in part) return roots.filter((el) => matches(clean(el.innerText || el.textContent), part.hasText, false));
    if ('hasNotText' in part) return roots.filter((el) => !matches(clean(el.innerText || el.textContent), part.hasNotText, false));
    if ('visible' in part) return roots.filter((el) => visible(el) === part.visible);
    throw new Error('unknown locator step ' + JSON.stringify(part));
  };
  const resolve = (chain) => {
    let roots = [document];
    for (const part of chain) roots = [...new Set(step(roots, part))];
    return roots;
  };
  const describe = (el) => {
    const rect = el.getBoundingClientRect();
    return {
      visible: visible(el),
      enabled: !el.disabled && el.getAttribute('aria-disabled') !== 'true',
      editable: el.isContentEditable || (['INPUT', 'TEXTAREA'].includes(el.tagName) && !el.readOnly && !el.disabled),
      x: rect.left + rect.width / 2,
      y: rect.top + rect.height / 2,
    };
  };
  const one = (chain) => {
    const found = resolve(chain);
    if (found.length !== 1) return { count: found.length };
    return { count: 1, el: found[0] };
  };
  return {
    count: (chain) => resolve(chain).length,
    probe: (chain, scroll) => {
      const found = one(chain);
      if (!found.el) return { count: found.count };
      if (scroll) found.el.scrollIntoView({ block: 'center', inline: 'center', behavior: 'instant' });
      return { count: 1, ...describe(found.el) };
    },
    focus: (chain, select) => {
      const { el } = one(chain);
      if (!el) return false;
      el.focus();
      if (select) {
        if (el.isContentEditable) {
          const range = document.createRange();
          range.selectNodeContents(el);
          const selection = getSelection();
          selection.removeAllRanges();
          selection.addRange(range);
        } else if (typeof el.select === 'function') {
          el.select();
        }
      }
      return true;
    },
    read: (chain, what, arg) => {
      if (what === 'all') return resolve(chain).map((el) => (arg === 'inner' ? el.innerText : el.textContent));
      const { el, count } = one(chain);
      if (!el) return { missing: count };
      const value = {
        text: () => el.textContent,
        inner: () => el.innerText,
        html: () => el.innerHTML,
        value: () => ('value' in el ? el.value : null),
        attribute: () => el.getAttribute(arg),
        visible: () => visible(el),
        checked: () => !!el.checked || el.getAttribute('aria-checked') === 'true',
        enabled: () => describe(el).enabled,
        editable: () => describe(el).editable,
      }[what];
      return { value: value() };
    },
    select: (chain, wanted) => {
      const { el, count } = one(chain);
      if (!el) return { missing: count };
      if (el.tagName !== 'SELECT') return { error: 'not a <select>' };
      const chosen = [];
      for (const option of el.options) {
        const pick = wanted.some((want) => option.value === want || clean(option.label) === want || clean(option.textContent) === want);
        option.selected = pick;
        if (pick) chosen.push(option.value);
      }
      if (!chosen.length) return { error: 'no option matches ' + JSON.stringify(wanted) };
      el.dispatchEvent(new Event('input', { bubbles: true }));
      el.dispatchEvent(new Event('change', { bubbles: true }));
      return { value: chosen };
    },
    call: (chain, fn, arg, every) => {
      if (every) return fn(resolve(chain), arg);
      const { el, count } = one(chain);
      if (!el) return Promise.reject(new Error('the locator matched ' + count + ' elements'));
      return fn(el, arg);
    },
  };
})()
