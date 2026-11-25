(cmd, card, arg) => {
  const typing = ['', 'text', 'email', 'tel', 'number', 'search', 'url'];
  const pressable = 'button,a[href],input[type=submit],input[type=button],input[type=image],[role=button],[role=link],summary';
  const wanted = { text: 'a text field', secret: 'a password or text field', code: 'a text field', check: 'a checkbox', press: 'a button or link' };
  const shown = (el) => {
    const style = getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden') return false;
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  };
  if (cmd === 'check') {
    const refs = globalThis.__refs;
    if (!refs) return { error: 'no element has a number yet; read the page first' };
    const els = new Map();
    const checks = [];
    for (const { ref, role } of arg) {
      const el = refs.get(ref);
      if (!el || !el.isConnected) return { error: '[' + ref + '] is not on the page now; read the page again' };
      const tag = el.tagName.toLowerCase();
      const type = tag === 'input' ? (el.getAttribute('type') || '').toLowerCase() : '';
      const text = tag === 'textarea' || (tag === 'input' && typing.includes(type));
      const password = tag === 'input' && type === 'password';
      const what = tag + (type ? ' ' + type : '');
      if (role === 'text' && password) return { error: '[' + ref + '] is a password field; give it the kind secret' };
      const fits = {
        text: text,
        secret: text || password,
        code: text || password,
        check: (tag === 'input' && type === 'checkbox') || el.getAttribute('role') === 'checkbox',
        press: el.matches(pressable),
      }[role];
      if (!fits) return { error: '[' + ref + '] is ' + what + ', not ' + wanted[role] };
      if (role !== 'check' && !shown(el)) return { error: '[' + ref + '] is hidden on the page' };
      if (['text', 'secret', 'code'].includes(role) && (el.disabled || el.readOnly)) return { error: '[' + ref + '] does not take text now' };
      els.set(ref, el);
      if (role === 'check') checks.push({ ref, on: el.checked === undefined ? el.getAttribute('aria-checked') === 'true' : !!el.checked });
    }
    globalThis.__fill = { card, els, origin: location.origin };
    return { origin: location.origin, host: location.host, checks };
  }
  const held = globalThis.__fill;
  if (!held || held.card !== card || held.origin !== location.origin) return { error: 'changed' };
  const el = held.els.get(arg);
  if (!el || !el.isConnected) return { error: 'changed' };
  if (cmd === 'at') {
    const target = shown(el) ? el : (el.labels && el.labels[0]) || el.parentElement || el;
    target.scrollIntoView({ block: 'center', inline: 'center' });
    const rect = target.getBoundingClientRect();
    return { x: rect.left + rect.width / 2, y: rect.top + rect.height / 2 };
  }
  if (cmd === 'focus') {
    el.focus();
    if (el.select) el.select();
    return { focused: document.activeElement === el };
  }
  if (cmd === 'checked') {
    return { checked: el.checked === undefined ? el.getAttribute('aria-checked') === 'true' : !!el.checked };
  }
  return { error: 'unknown' };
}
