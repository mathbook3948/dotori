function (command, arg, ...els) {
  const typing = ['', 'text', 'email', 'tel', 'number', 'search', 'url'];
  const clickable = 'button,a[href],input[type=submit],input[type=button],input[type=image],[role=button],[role=link],summary';
  const wanted = { text: 'a text field', secret: 'a password field', code: 'a text or password field', check: 'a checkbox', submit: 'a button', link: 'a button or link' };
  const prop = (el, name) => {
    for (let proto = Object.getPrototypeOf(el); proto; proto = Object.getPrototypeOf(proto)) {
      const found = Object.getOwnPropertyDescriptor(proto, name);
      if (found && found.get) return found.get.call(el);
    }
    return undefined;
  };
  const has = (el, name) => Element.prototype.hasAttribute.call(el, name);
  const shown = (el) => {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  const originOf = (url) => {
    try {
      return new URL(url, location.href).origin;
    } catch (error) {
      return '';
    }
  };
  const fields = () => Array.from(document.querySelectorAll('input, textarea, select'));
  if (command === 'remember') {
    globalThis.__before = new Map(fields().map((el) => [el, prop(el, 'value')]));
    return {};
  }
  if (command === 'changed') {
    const before = globalThis.__before || new Map();
    return fields().filter((el) => before.get(el) !== prop(el, 'value') && (before.has(el) || prop(el, 'value')));
  }
  if (command === 'check') {
    if (location.protocol !== 'https:' || !location.host) return { origin: location.origin, host: location.host, checks: [] };
    if (navigator.serviceWorker && navigator.serviceWorker.controller) return { error: 'a service worker runs this page, so dotori fills nothing on it' };
    const checks = [];
    const sent = [];
    for (let n = 0; n < arg.roles.length; n++) {
      const { ref, role } = arg.roles[n];
      const el = els[n];
      if (!el || !el.isConnected) return { error: '[' + ref + '] is not on the page now; read the page again' };
      const tag = el.tagName.toLowerCase();
      const type = tag === 'input' ? (el.getAttribute('type') || '').toLowerCase() : '';
      const text = tag === 'textarea' || (tag === 'input' && typing.includes(type));
      const password = tag === 'input' && type === 'password';
      const what = tag + (type ? ' ' + type : '');
      if (role === 'text' && password) return { error: '[' + ref + '] is a password field; give it the kind secret' };
      const fits = {
        text: text,
        secret: password,
        code: text || password,
        check: (tag === 'input' && type === 'checkbox') || el.getAttribute('role') === 'checkbox',
        submit: el.matches(clickable),
        link: el.matches(clickable),
      }[role];
      if (!fits) return { error: '[' + ref + '] is ' + what + ', not ' + wanted[role] };
      if (role !== 'check' && !shown(el)) return { error: '[' + ref + '] is hidden on the page' };
      if (['text', 'secret', 'code'].includes(role) && (el.disabled || el.readOnly)) return { error: '[' + ref + '] does not take text now' };
      if (role === 'check') checks.push({ ref, on: el.checked === undefined ? el.getAttribute('aria-checked') === 'true' : !!el.checked });
      if (['text', 'secret', 'code', 'submit'].includes(role)) sent.push({ ref, el, role });
    }
    const formOf = (el) => (['input', 'textarea', 'select', 'button'].includes(el.tagName.toLowerCase()) ? prop(el, 'form') || null : null);
    const forms = new Set(sent.map(({ el }) => formOf(el)));
    if (forms.size > 1) return { error: 'the fields and the button are not in one form on the page' };
    const form = sent.length ? formOf(sent[0].el) : null;
    if (form) {
      let action = prop(form, 'action');
      let method = prop(form, 'method');
      for (const { el, role } of sent) {
        if (role !== 'submit' || !['button', 'input'].includes(el.tagName.toLowerCase())) continue;
        if (has(el, 'formaction')) action = prop(el, 'formAction');
        if (has(el, 'formmethod')) method = prop(el, 'formMethod');
      }
      if (originOf(action) !== location.origin) return { error: 'the form sends what is typed to another site than this page' };
      const secret = sent.some(({ role }) => role === 'secret' || role === 'code');
      if (secret && String(method).toLowerCase() !== 'post') return { error: 'the form would put the password or code in the address, so dotori fills nothing on it' };
    }
    return { origin: location.origin, host: location.host, path: location.pathname, checks };
  }
  const el = els[0];
  if (!el || !el.isConnected) return { error: 'changed' };
  if (command === 'at') {
    const target = shown(el) ? el : (el.labels && el.labels[0]) || el.parentElement || el;
    target.scrollIntoView({ block: 'center', inline: 'center' });
    const rect = target.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  }
  if (command === 'focus') {
    el.focus();
    if (el.select) el.select();
    return { focused: el.getRootNode().activeElement === el };
  }
  if (command === 'focused') return { focused: el.getRootNode().activeElement === el };
  if (command === 'landed') return { landed: prop(el, 'value') === arg };
  if (command === 'checked') {
    return { checked: el.checked === undefined ? el.getAttribute('aria-checked') === 'true' : !!el.checked };
  }
  if (command === 'clear') {
    el.value = '';
    el.dispatchEvent(new Event('input', { bubbles: true }));
    el.dispatchEvent(new Event('change', { bubbles: true }));
    return {};
  }
  return { error: 'unknown' };
}
