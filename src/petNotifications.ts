export function showPetNotification(element: HTMLElement): void {
  if (!element.classList.contains('hidden')) return;
  element.parentElement?.prepend(element);
  element.classList.remove('hidden');
}
