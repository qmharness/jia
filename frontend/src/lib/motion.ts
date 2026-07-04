/** CSS transition string for FLIP animations */
export function flipTransition(
  duration = '250ms',
  easing = 'cubic-bezier(0.34, 1.56, 0.64, 1)'
): string {
  return `transform ${duration} ${easing}, opacity ${duration} ${easing}`;
}

/** Staggered delay for list items */
export function staggerDelay(index: number, baseMs = 50): string {
  return `${index * baseMs}ms`;
}
