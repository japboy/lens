// jsdom does not implement layout. This observer permits component contract tests;
// mounted ranges and variable-height scrolling require real WebView qualification.
if (typeof window !== "undefined" && !window.ResizeObserver) {
  window.ResizeObserver = class implements ResizeObserver {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  };
}
