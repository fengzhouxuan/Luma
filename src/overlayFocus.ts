import { useLayoutEffect, useRef, type RefObject } from "react";

export const focusableSelector = 'button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), summary, [tabindex]:not([tabindex="-1"])';
export function focusableElements(scope: HTMLElement): HTMLElement[] {
  return [...scope.querySelectorAll<HTMLElement>(focusableSelector)]
    .filter(element => element.tabIndex >= 0 && !element.closest('[inert]') && element.getClientRects().length > 0);
}

// One owner for the topmost overlay, including nested import/editor panels and menus.
export function useOverlayFocus(root: RefObject<HTMLElement | null>, key: string | null,
  opener: RefObject<HTMLElement | null>, fallback: RefObject<HTMLElement | null>) {
  const previous = useRef<string | null>(null);
  useLayoutEffect(() => {
    const scopes = [...(root.current?.querySelectorAll<HTMLElement>('[data-focus-scope]') ?? [])];
    const scope = scopes[scopes.length - 1];
    if (!key || !scope) {
      if (previous.current) {
        const target = opener.current;
        (target?.isConnected && !target.closest('[inert]') ? target : fallback.current)?.focus({ preventScroll: true });
      }
      previous.current = key;
      return;
    }
    previous.current = key;
    const lower = scopes.slice(0, -1).map(element => ({ element, inert: element.inert }));
    lower.forEach(({ element }) => { element.inert = true; });
    const focusFirst = () => {
      const preferred = scope.querySelector<HTMLElement>('[data-initial-focus]:not(:disabled)');
      (preferred ?? focusableElements(scope)[0] ?? scope).focus({ preventScroll: true });
    };
    focusFirst();
    const guard = (event: FocusEvent) => { if (!scope.contains(event.target as Node)) focusFirst(); };
    const trap = (event: KeyboardEvent) => {
      if (event.key !== 'Tab') return;
      const items = focusableElements(scope);
      const index = items.indexOf(document.activeElement as HTMLElement);
      if (index < 0 || (event.shiftKey ? index === 0 : index === items.length - 1)) {
        event.preventDefault();
        event.stopPropagation();
        (event.shiftKey ? items[items.length - 1] ?? scope : items[0] ?? scope).focus();
      }
    };
    document.addEventListener('focusin', guard);
    document.addEventListener('keydown', trap, true);
    return () => {
      document.removeEventListener('focusin', guard);
      document.removeEventListener('keydown', trap, true);
      lower.forEach(({ element, inert }) => { element.inert = inert; });
    };
  }, [root, key, opener, fallback]);
}
