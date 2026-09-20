import { controlStyles } from "../styles/component-styles";
import { LitElement, css, html, nothing, type PropertyValues } from "lit";
import { customElement, property, state } from "lit/decorators.js";
import { styleMap } from "lit/directives/style-map.js";

export interface SelectOption {
  value: string;
  label: string;
  icon?: string;
  disabled?: boolean;
  group?: string;
  description?: string;
}
let nextId = 0;

const lensSelectStyles = css`
  :host {
    display: block;
    position: relative;
    min-width: 0;
  }
  button {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    box-sizing: border-box;
    padding: 3px 8px;
    text-align: start;
    font: inherit;
  }
  .lens-select-chevron {
    width: 0.4em;
    height: 0.4em;
    flex: 0 0 0.4em;
    border-right: 1.5px solid currentColor;
    border-bottom: 1.5px solid currentColor;
    transform: translateY(-0.15em) rotate(45deg);
    margin-inline: 3px;
  }
  .lens-select-group {
    padding: 6px 8px 3px;
    font-size: 0.85em;
    font-weight: 600;
    opacity: 0.7;
  }
  .lens-select-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .lens-select-icon {
    display: inline-block;
    width: 1em;
    height: 1em;
    flex: 0 0 1em;
    background: currentColor;
    mask: no-repeat center / contain;
    -webkit-mask: no-repeat center / contain;
  }
  .lens-select-popup {
    position: fixed;
    margin: 0;
    padding: 4px;
    box-sizing: border-box;
    overflow: auto;
    overscroll-behavior: contain;
    border: 1px solid color-mix(in srgb, currentColor 20%, transparent);
    border-radius: 8px;
    background: Canvas;
    color: CanvasText;
    box-shadow: 0 8px 24px #0003;
    font: inherit;
    z-index: 10;
  }
  [role="option"] {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px;
    border-radius: 4px;
    cursor: default;
  }
  [role="option"].active {
    background: Highlight;
    color: HighlightText;
  }
  [aria-disabled="true"] {
    opacity: 0.5;
  }
  .lens-select-error {
    display: block;
    color: var(--error-color, #b3261e);
    font-size: 0.85em;
  }
  .lens-select-done {
    width: 100%;
    margin-top: 4px;
  }
  .lens-select-check {
    width: 1em;
  }
`;

/** Self-contained select-only combobox. Highlighting never changes the committed value. */
@customElement("lens-select")
export class LensSelect extends LitElement {
  static styles = [controlStyles, lensSelectStyles];
  @property({ attribute: false }) options: readonly SelectOption[] = [];
  @property() value = "";
  @property() label = "";
  @property() name = "";
  @property({ type: Boolean }) multiple = false;
  @property({ attribute: false }) values: readonly string[] = [];
  @property({ type: Boolean }) required = false;
  @state() private invalid = false;
  @property({ type: Boolean }) disabled = false;
  @state() private expanded = false;
  @state() private active = -1;
  @state() private placement: Record<string, string> = {};
  @state() private listId = "";
  private scrollRoot: ShadowRoot | undefined;
  private typed = "";
  private typedAt = 0;

  connectedCallback(): void {
    if (!this.listId) this.listId = `lens-select-list-${++nextId}`;
    super.connectedCallback();
    this.ownerDocument.addEventListener("pointerdown", this.outside, true);
    this.ownerDocument.addEventListener("focusin", this.outside, true);
    this.ownerDocument.addEventListener("scroll", this.scrolled, true);
    const root = this.getRootNode();
    if (root instanceof ShadowRoot) {
      this.scrollRoot = root;
      root.addEventListener("scroll", this.scrolled, true);
    }
    this.ownerDocument.defaultView?.addEventListener("resize", this.resized);
  }
  disconnectedCallback(): void {
    this.ownerDocument.removeEventListener("pointerdown", this.outside, true);
    this.ownerDocument.removeEventListener("focusin", this.outside, true);
    this.ownerDocument.removeEventListener("scroll", this.scrolled, true);
    this.scrollRoot?.removeEventListener("scroll", this.scrolled, true);
    this.scrollRoot = undefined;
    this.ownerDocument.defaultView?.removeEventListener("resize", this.resized);
    this.cancel();
    super.disconnectedCallback();
  }
  private outside = (event: Event): void => {
    if (!event.composedPath().includes(this)) this.cancel();
  };
  private scrolled = (event: Event): void => {
    if (!event.composedPath().includes(this)) this.cancel();
  };
  private resized = (): void => this.cancel();
  private cancel(): void {
    this.expanded = false;
    this.active = -1;
    this.typed = "";
  }
  protected willUpdate(changed: PropertyValues<this>): void {
    if (this.disabled || (this.expanded && (changed.has("value") || changed.has("options"))))
      this.cancel();
    if (this.expanded && !this.enabled(this.active)) this.active = this.initial();
    if (this.invalid && this.checkValidity()) this.invalid = false;
  }
  private get unavailable(): boolean {
    return this.disabled || Boolean(this.closest("fieldset[disabled]"));
  }
  private selectedValues(): string[] {
    return this.options
      .filter((option) => !option.disabled && this.selected(option))
      .map((option) => option.value);
  }
  private selected(option: SelectOption): boolean {
    return this.multiple ? this.values.includes(option.value) : option.value === this.value;
  }
  public checkValidity(): boolean {
    return (
      this.unavailable || !this.required || this.selectedValues().some((value) => value !== "")
    );
  }
  public formEntries(): [string, string][] {
    return !this.name || this.unavailable
      ? []
      : this.selectedValues().map((value) => [this.name, value]);
  }
  public reportValidity(): boolean {
    const valid = this.checkValidity();
    this.invalid = !valid;
    if (!valid) this.renderRoot.querySelector<HTMLButtonElement>("button")?.focus();
    return valid;
  }
  private enabled(index: number): boolean {
    return this.options[index] !== undefined && !this.options[index]!.disabled;
  }
  private initial(): number {
    const selected = this.options.findIndex((option) => this.selected(option) && !option.disabled);
    return selected >= 0 ? selected : this.options.findIndex((option) => !option.disabled);
  }
  private show(): void {
    if (this.unavailable) return;
    const button = this.renderRoot.querySelector<HTMLButtonElement>("button")!;
    // Safari does not focus buttons on pointer activation; keyboard navigation
    // belongs to this combobox trigger while the popup is open.
    button.focus({ preventScroll: true });
    const rect = button.getBoundingClientRect();
    const viewport = this.ownerDocument.defaultView;
    const height = viewport?.innerHeight ?? 600;
    const below = height - rect.bottom - 8;
    const above = rect.top - 8;
    const upwards = below < 160 && above > below;
    this.placement = {
      left: `${Math.max(8, rect.left)}px`,
      top: upwards ? "auto" : `${rect.bottom + 4}px`,
      bottom: upwards ? `${height - rect.top + 4}px` : "auto",
      width: "max-content",
      "min-width": `${Math.min(rect.width, (viewport?.innerWidth ?? 800) - 16)}px`,
      "max-width": `${Math.max(0, (viewport?.innerWidth ?? 800) - 16)}px`,
      "max-height": `${Math.max(40, Math.min(320, upwards ? above : below))}px`,
    };
    this.active = this.initial();
    this.expanded = true;
  }
  protected updated(): void {
    const popup = this.renderRoot.querySelector<HTMLElement>(".lens-select-popup");
    if (this.expanded) {
      popup?.showPopover?.();
      if (popup) {
        const viewportWidth = this.ownerDocument.defaultView?.innerWidth ?? 800;
        const width = popup.getBoundingClientRect().width;
        const left = Math.max(
          8,
          Math.min(Number.parseFloat(this.placement.left ?? "8"), viewportWidth - width - 8),
        );
        if (this.placement.left !== `${left}px`) {
          this.placement = { ...this.placement, left: `${left}px` };
        }
      }
      this.renderRoot
        .querySelector<HTMLElement>(`[data-index="${this.active}"]`)
        ?.scrollIntoView?.({
          block: "nearest",
        });
    } else {
      popup?.hidePopover?.();
    }
  }
  private commit(index: number): void {
    if (this.unavailable || !this.enabled(index)) return;
    const value = this.options[index]!.value;
    if (this.multiple) {
      this.values = this.values.includes(value)
        ? this.values.filter((selected) => selected !== value)
        : [...this.values, value];
      this.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    } else if (value !== this.value) {
      this.value = value;
      this.dispatchEvent(new Event("change", { bubbles: true, composed: true }));
    }
    if (!this.multiple) this.cancel();
    this.renderRoot.querySelector<HTMLButtonElement>("button")?.focus();
  }
  private move(direction: number, edge = false): void {
    const start = edge ? (direction > 0 ? -1 : this.options.length) : this.active;
    for (
      let index = start + direction;
      index >= 0 && index < this.options.length;
      index += direction
    ) {
      if (this.enabled(index)) {
        this.active = index;
        break;
      }
    }
  }
  private keydown(event: KeyboardEvent): void {
    if (this.unavailable) return;
    const wasOpen = this.expanded;
    if (["ArrowDown", "ArrowUp", "Home", "End"].includes(event.key)) {
      event.preventDefault();
      if (!wasOpen) this.show();
      if (event.key === "Home") this.move(1, true);
      else if (event.key === "End") this.move(-1, true);
      else if (wasOpen) this.move(event.key === "ArrowDown" ? 1 : -1);
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      if (wasOpen && this.multiple && event.key === "Enter") this.cancel();
      else if (wasOpen) this.commit(this.active);
      else this.show();
    } else if (event.key === "Escape") {
      if (wasOpen) event.preventDefault();
      this.cancel();
    } else if (event.key === "Tab") {
      this.cancel();
    } else if (event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
      event.preventDefault();
      if (!wasOpen) this.show();
      const now = Date.now();
      this.typed = (
        now - this.typedAt < 700 ? this.typed + event.key.toLowerCase() : event.key.toLowerCase()
      ).slice(0, 128);
      this.typedAt = now;
      const query = [...this.typed].every((char) => char === this.typed[0])
        ? this.typed[0]!
        : this.typed;
      for (let offset = 1; offset <= this.options.length; offset++) {
        const index = (this.active + offset) % this.options.length;
        if (this.enabled(index) && this.options[index]!.label.toLowerCase().startsWith(query)) {
          this.active = index;
          break;
        }
      }
    }
  }
  private icon(url: string | undefined) {
    return url
      ? html`<span
          class="lens-select-icon"
          aria-hidden="true"
          style=${styleMap({ "mask-image": `url(${JSON.stringify(url)})`, "-webkit-mask-image": `url(${JSON.stringify(url)})` })}
        ></span>`
      : nothing;
  }
  protected render() {
    const selected = this.options.filter((option) => this.selected(option));
    return html` <button
        type="button"
        role="combobox"
        aria-label=${this.label}
        aria-haspopup="listbox"
        aria-expanded=${String(this.expanded)}
        aria-controls=${this.listId || nothing}
        aria-required=${String(this.required)}
        aria-invalid=${String(this.invalid)}
        aria-describedby=${this.invalid ? `${this.listId}-error` : nothing}
        aria-activedescendant=${this.expanded && this.active >= 0 ? `${this.listId}-${this.active}` : nothing}
        ?disabled=${this.disabled}
        @keydown=${this.keydown}
        @click=${() => (this.expanded ? this.cancel() : this.show())}
      >
        ${this.icon(selected.length === 1 ? selected[0]?.icon : undefined)}<span
          class="lens-select-label"
          >${selected.map((option) => option.label).join(", ") || "Choose…"}</span
        >
        <span class="lens-select-chevron" aria-hidden="true"></span>
      </button>
      <div
        class="lens-select-popup"
        popover="manual"
        style=${styleMap({ ...this.placement, display: this.expanded ? "block" : "none" })}
        @pointerdown=${(event: PointerEvent) => event.preventDefault()}
      >
        <div
          id=${this.listId || nothing}
          role="listbox"
          aria-label=${this.label}
          aria-multiselectable=${String(this.multiple)}
        >
          ${this.options.map(
            (
              option,
              index,
            ) => html`${option.group && option.group !== this.options[index - 1]?.group ? html`<div class="lens-select-group" aria-hidden="true">${option.group}</div>` : nothing}
              <div
                role="option"
                aria-label=${option.group ? `${option.group}: ${option.label}` : nothing}
                title=${option.description ?? nothing}
                id=${this.listId ? `${this.listId}-${index}` : nothing}
                data-index=${index}
                aria-selected=${String(this.selected(option))}
                aria-disabled=${String(Boolean(option.disabled))}
                class=${index === this.active ? "active" : ""}
                @pointermove=${() => {
                  if (this.enabled(index)) this.active = index;
                }}
                @click=${() => this.commit(index)}
              >
                ${this.icon(option.icon)}<span class="lens-select-label">${option.label}</span>
                <span class="lens-select-check" aria-hidden="true"
                  >${this.selected(option) ? "✓" : ""}</span
                >
              </div>`,
          )}
        </div>
        ${
          this.multiple
            ? html`<button
                type="button"
                tabindex="-1"
                class="lens-select-done"
                @click=${() => {
                  this.cancel();
                  this.renderRoot.querySelector<HTMLButtonElement>("button")?.focus();
                }}
              >
                Done
              </button>`
            : nothing
        }
      </div>
      ${this.invalid ? html`<span id=${`${this.listId}-error`} class="lens-select-error" role="alert">Choose at least one option.</span>` : nothing}`;
  }
}
