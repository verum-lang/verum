import { glyphs } from './icons-data';
export type IconName = keyof typeof glyphs;
const ns = 'http://www.w3.org/2000/svg';
export function icon(name: IconName, className = ''): SVGSVGElement {
  const svg = document.createElementNS(ns, 'svg');
  svg.setAttribute('viewBox', '0 0 24 24'); svg.setAttribute('fill', 'none');
  svg.setAttribute('width', '18'); svg.setAttribute('height', '18');
  svg.setAttribute('class', `icon ${className}`); svg.setAttribute('aria-hidden', 'true'); svg.setAttribute('focusable', 'false');
  for (const [tag, attributes] of glyphs[name]) {
    const child = document.createElementNS(ns, tag);
    for (const [key, value] of Object.entries(attributes)) if (key !== 'key') child.setAttribute(key.replace(/[A-Z]/g, letter => `-${letter.toLowerCase()}`), value);
    svg.append(child);
  }
  return svg;
}
export function installIcons() {
  for (const placeholder of document.querySelectorAll<HTMLElement>('[data-icon]')) {
    const name = placeholder.dataset.icon as IconName;
    if (name in glyphs) placeholder.replaceWith(icon(name, placeholder.className));
  }
}
