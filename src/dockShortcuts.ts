interface ShortcutEvent { key: string; code?: string; altKey: boolean; ctrlKey: boolean; shiftKey: boolean; metaKey: boolean; repeat: boolean; isComposing: boolean }
export function dockShortcutIndex(event: ShortcutEvent): number | null {
  if (!event.altKey || event.ctrlKey || event.shiftKey || event.metaKey || event.repeat || event.isComposing) return null;
  const digit = /^[1-9]$/.test(event.key) ? event.key : /^(?:Digit|Numpad)[1-9]$/.test(event.code ?? "") ? event.code!.slice(-1) : null;
  return digit === null ? null : Number(digit) - 1;
}
