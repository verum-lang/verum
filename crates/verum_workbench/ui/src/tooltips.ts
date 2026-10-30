/** One keyboard-accessible tooltip layer; no per-control layout or hidden text. */
export function installTooltips() {
  for (const button of document.querySelectorAll<HTMLButtonElement>('button[title]')) {
    button.dataset.tooltip = button.title; button.removeAttribute('title');
  }
  const tooltip = document.createElement('div'); tooltip.id = 'control-tooltip'; tooltip.className = 'control-tooltip'; tooltip.setAttribute('role', 'tooltip'); tooltip.hidden = true; document.body.append(tooltip);
  let current: HTMLElement | null = null, timer = 0;
  function hide() {
    window.clearTimeout(timer); tooltip.hidden = true;
    if (current) { const other = (current.getAttribute('aria-describedby') || '').split(' ').filter(id => id && id !== tooltip.id); if (other.length) current.setAttribute('aria-describedby', other.join(' ')); else current.removeAttribute('aria-describedby'); }
    current = null;
  }
  function show(target: EventTarget | null, keyboard: boolean) {
    const control = target instanceof Element ? target.closest<HTMLElement>('button, [role="button"]') : null;
    if (!control || control === current) return;
    if (keyboard && !control.matches(':focus-visible')) return;
    const text = control.dataset.tooltip || control.title || control.getAttribute('aria-label');
    if (!text) return; hide(); current = control;
    timer = window.setTimeout(() => {
      if (current !== control || !control.isConnected) return;
      (control.closest('dialog') || document.body).append(tooltip); tooltip.textContent = text; tooltip.hidden = false;
      const bounds = control.getBoundingClientRect();
      tooltip.style.left = `${Math.max(8, Math.min(innerWidth - tooltip.offsetWidth - 8, bounds.left + bounds.width / 2 - tooltip.offsetWidth / 2))}px`;
      tooltip.style.top = `${bounds.bottom + tooltip.offsetHeight + 8 <= innerHeight ? bounds.bottom + 6 : Math.max(8, bounds.top - tooltip.offsetHeight - 6)}px`;
      control.setAttribute('aria-describedby', `${control.getAttribute('aria-describedby') || ''} ${tooltip.id}`.trim());
    }, keyboard ? 0 : 350);
  }
  document.addEventListener('pointerover', event => show(event.target, false));
  document.addEventListener('focusin', event => show(event.target, true));
  document.addEventListener('pointerout', event => { if (current && (!(event.relatedTarget instanceof Node) || !current.contains(event.relatedTarget))) hide(); });
  document.addEventListener('focusout', hide); document.addEventListener('pointerdown', hide);
  document.addEventListener('scroll', hide, true); window.addEventListener('blur', hide);
  document.addEventListener('studio:language', hide);
  document.addEventListener('keydown', event => { if (event.key === 'Escape') hide(); });
}
